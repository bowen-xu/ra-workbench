//! One typed transaction for user queue edits; execution admission stays separate.
use super::*;
use crate::agent::repository::{SqlRepositoryError, input_queue};
use research_agent_core::{conversation::CatalogError, input_queue::InputQueue};
use rusqlite::Connection;
#[path = "queue_edit/types.rs"]
mod types;
pub(crate) use types::{
    QUEUE_EDIT_FAMILY, QueueEditFailure, QueueEditIntent, QueueEditOutcome, QueueEditScope,
    QueueEditStatus,
};
#[path = "queue_edit/authorization.rs"]
pub(super) mod authorization;
#[path = "queue_edit/status.rs"]
mod status;

enum Control {
    Domain(input_queue::Error),
    Legacy(InputQueue),
}
fn signal(control: &mut Option<Control>, value: Control) -> SqlImmediateFailure {
    *control = Some(value);
    // Private control value only. execute_sql must confirm ROLLBACK before the
    // family can restore a fine-grained refusal or its read-only legacy replay.
    SqlImmediateFailure::Domain(CatalogError::InvalidInput)
}
fn raw<T>(
    result: Result<T, input_queue::QueueSqlError>,
    intent: &QueueEditIntent,
    fence: ReadFence,
    phase: &'static str,
    control: &mut Option<Control>,
) -> Result<T, SqlImmediateFailure> {
    result.map_err(|cause| match cause {
        SqlRepositoryError::Sql(cause) => {
            receipts::storage(intent.request_id, fence, phase, &cause)
        }
        SqlRepositoryError::Domain(input_queue::Error::Corrupt) => {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                intent.request_id,
                fence,
                phase,
                "queue_edit_state_corrupt",
            )))
        }
        SqlRepositoryError::Domain(input_queue::Error::Storage) => {
            SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                intent.request_id,
                fence,
                phase,
                "queue_edit_storage_unavailable",
            )))
        }
        SqlRepositoryError::Domain(cause) => signal(control, Control::Domain(cause)),
    })
}
fn deleted(db: &Connection, conversation: Uuid) -> rusqlite::Result<bool> {
    db.query_row("select exists(select 1 from agent_conversation_deletions where conversation_id=?1) and not exists(select 1 from agent_conversations where id=?1)",
        [conversation.to_string()], |row| row.get(0))
}
fn frozen(
    db: &Connection,
    intent: &QueueEditIntent,
    fence: ReadFence,
) -> Result<Option<InputQueue>, SqlImmediateFailure> {
    input_queue::read_frozen_edit_sql(
        db,
        intent.conversation_id,
        intent.request_id,
        intent.expected_revision,
        intent.lowered(),
    )
    .map_err(|cause| match cause {
        SqlRepositoryError::Sql(cause) => {
            receipts::storage(intent.request_id, fence, "queue-edit-replay", &cause)
        }
        SqlRepositoryError::Domain(_) => SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            intent.request_id,
            fence,
            "queue-edit-replay",
            "queue_edit_frozen_receipt_invalid",
        ))),
    })
}
fn register(
    tx: &Transaction<'_>,
    intent: &QueueEditIntent,
    identity: &ReceiptIdentity,
    fence: ReadFence,
    queue: &mut Option<InputQueue>,
    control: &mut Option<Control>,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    if intent
        .expected_epoch
        .is_some_and(|epoch| epoch != fence.epoch)
    {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            intent.request_id,
            fence,
            "queue-edit-epoch",
            "data_epoch_changed",
        ))));
    }
    receipts::check_state(tx, intent.request_id, fence, "queue-edit")?;
    receipts::check_cross_kind(tx, intent.request_id, fence, "queue-edit")?;
    let receipt = receipts::lookup(tx, identity, fence, "queue-edit")?;
    // Preserve the existing current-authorization-before-replay write contract.
    raw(
        authorization::authorize(tx, intent),
        intent,
        fence,
        "queue-edit-authorize",
        control,
    )?;
    if let Some(receipt) = receipt {
        *queue = Some(frozen(tx, intent, fence)?.ok_or_else(|| {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                intent.request_id,
                fence,
                "queue-edit-replay",
                "queue_edit_business_receipt_missing",
            )))
        })?);
        return Ok(receipt);
    }
    let legacy = raw(
        input_queue::read_edit_receipt_sql(tx, intent.conversation_id, intent.request_id),
        intent,
        fence,
        "queue-edit-legacy",
        control,
    )?
    .is_some();
    let result = raw(
        input_queue::apply_in_tx_sql(
            tx,
            intent.conversation_id,
            intent.request_id,
            intent.expected_revision,
            intent.lowered(),
            |_| Ok(()),
        ),
        intent,
        fence,
        "queue-edit-mutation",
        control,
    )?;
    if legacy {
        return Err(signal(control, Control::Legacy(result)));
    }
    *queue = Some(result);
    receipts::insert(tx, identity, fence)
}
impl Database {
    pub(crate) fn edit_input_queue(
        &self,
        intent: &QueueEditIntent,
    ) -> Result<QueueEditOutcome, QueueEditFailure> {
        self.edit_input_queue_owned(
            intent,
            #[cfg(test)]
            Fault::None,
        )
    }
    fn edit_input_queue_owned(
        &self,
        intent: &QueueEditIntent,
        #[cfg(test)] fault: Fault,
    ) -> Result<QueueEditOutcome, QueueEditFailure> {
        let identity = intent.identity()?;
        let mut queue = None;
        let mut control = None;
        #[cfg(test)]
        let break_control_rollback = matches!(&fault, Fault::Rollback);
        let result = self.execute_sql(
            intent.request_id,
            |tx, fence| {
                let result = register(tx, intent, &identity, fence, &mut queue, &mut control);
                #[cfg(test)]
                if result.is_err() && control.is_some() && break_control_rollback {
                    // A real SQLite cleanup error verifies that confirmed domain /
                    // LegacyReplay results cannot mask the shell's rollback Unknown.
                    tx.execute_batch("rollback").unwrap();
                }
                result
            },
            #[cfg(test)]
            fault,
        );
        match result {
            Ok(receipt) => Ok(QueueEditOutcome::Committed {
                queue: queue.expect("successful edit transaction sets its queue"),
                receipt,
            }),
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput)) => match control {
                Some(Control::Domain(cause)) => Err(QueueEditFailure::Domain(cause)),
                Some(Control::Legacy(queue)) => Ok(QueueEditOutcome::LegacyReplay(queue)),
                None => Err(QueueEditFailure::Storage(SqlImmediateFailure::Domain(
                    CatalogError::InvalidInput,
                ))),
            },
            Err(cause) => Err(cause.into()),
        }
    }
}

#[cfg(test)]
#[path = "queue_edit/tests.rs"]
mod tests;
