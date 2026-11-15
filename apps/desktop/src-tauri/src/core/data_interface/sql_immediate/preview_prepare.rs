//! Typed ordinary prepare uses the existing journal admission and SQL owner.
use super::*;
use crate::agent::repository::input_queue::preview_prepare::{self as queue, Preparation};
use crate::{
    agent::writing_context::WritingContextReader, ai::structured_resource_validation::Flow,
};
pub(crate) use queue::{
    QueuePreviewPrepareIntent, QueuePreviewPrepareSource, QueuePreviewPrepared,
};
#[path = "preview_prepare/materialization_error.rs"]
mod materialization_error;
#[path = "preview_prepare/status.rs"]
mod status;

#[derive(Debug)]
pub(crate) enum QueuePreviewPrepareStatus {
    Committed {
        state: Box<QueuePreviewPrepared>,
        receipt: Box<SqlImmediateReceipt>,
    },
    NotFound,
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}

fn identity(command: &QueuePreviewPrepareIntent) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    let frozen = queue::command(command).map_err(|cause| match cause {
        SqlMutationError::Domain(cause) => SqlImmediateFailure::Domain(cause),
        SqlMutationError::Sql(_) => unreachable!("command encoding performs no SQL"),
    })?;
    Ok(ReceiptIdentity {
        operation_id: command.request_id,
        kind: "agent.queue.preview.prepare",
        version: 1,
        fingerprint: queue::command_fingerprint(&frozen),
    })
}
fn error(
    command: &QueuePreviewPrepareIntent,
    fence: ReadFence,
    phase: &'static str,
    cause: SqlMutationError,
) -> SqlImmediateFailure {
    match cause {
        SqlMutationError::Sql(cause) => receipts::storage(command.request_id, fence, phase, &cause),
        SqlMutationError::Domain(research_agent_core::conversation::CatalogError::Corrupt) => {
            corrupt(command, fence, phase, "queue_prepare_state_corrupt")
        }
        SqlMutationError::Domain(cause) => SqlImmediateFailure::Domain(cause),
    }
}
fn corrupt(
    command: &QueuePreviewPrepareIntent,
    fence: ReadFence,
    phase: &'static str,
    cause: &'static str,
) -> SqlImmediateFailure {
    SqlImmediateFailure::Corrupt(Box::new(diagnostic(
        command.request_id,
        fence,
        phase,
        cause,
    )))
}
fn replay(
    db: &rusqlite::Connection,
    command: &QueuePreviewPrepareIntent,
    fence: ReadFence,
    receipt: Option<SqlImmediateReceipt>,
) -> Result<Option<(QueuePreviewPrepared, SqlImmediateReceipt)>, SqlImmediateFailure> {
    queue::authorize(db, command)
        .map_err(|cause| error(command, fence, "queue-prepare-authorize", cause))?;
    let saved = queue::read_in(db, command)
        .map_err(|cause| error(command, fence, "queue-prepare-replay", cause))?;
    match (receipt, saved) {
        (Some(receipt), Preparation::Prepared(saved)) => Ok(Some((*saved, receipt))),
        (None, Preparation::Missing) => {
            queue::require_fresh(db, command)
                .map_err(|cause| error(command, fence, "queue-prepare-eligibility", cause))?;
            Ok(None)
        }
        (None, Preparation::Legacy) => Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            command.request_id,
            fence,
            "queue-prepare-replay",
            "legacy_prepare_without_intent_proof",
        )))),
        _ => Err(corrupt(
            command,
            fence,
            "queue-prepare-replay",
            "queue_and_sql_receipts_disagree",
        )),
    }
}

impl Database {
    /// A write-admitted replay probe precedes model resolution and source I/O.
    /// It cannot unblock the owner or grant new use of a revoked binding.
    pub(crate) fn replay_queue_preview_prepare(
        &self,
        command: &QueuePreviewPrepareIntent,
    ) -> Result<Option<QueuePreviewPrepared>, SqlImmediateFailure> {
        let identity = identity(command)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.request_id))?;
        let mut admission = owner
            .materialize_guard()
            .map_err(|cause| admission_error(command.request_id, cause, None))?;
        let fence = admission.fence();
        let db = self.raw.lock().map_err(|_| {
            admission.block();
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-prepare-replay",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            admission.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-prepare-replay",
                "sql_transaction_already_open",
            ))));
        }
        let result = (|| {
            receipts::check_state(&db, command.request_id, fence, "queue-prepare-replay")?;
            receipts::check_cross_kind(&db, command.request_id, fence, "queue-prepare-replay")?;
            let receipt = receipts::lookup(&db, &identity, fence, "queue-prepare-replay")?;
            replay(&db, command, fence, receipt).map(|saved| saved.map(|(state, _)| state))
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            admission.block();
        }
        result
    }

    pub(crate) fn prepare_queue_preview(
        &self,
        command: &QueuePreviewPrepareIntent,
        source: QueuePreviewPrepareSource,
    ) -> Result<QueuePreviewPrepared, SqlImmediateFailure> {
        self.prepare_queue_preview_owned(
            command,
            source,
            #[cfg(test)]
            Fault::None,
        )
    }
    fn prepare_queue_preview_owned(
        &self,
        command: &QueuePreviewPrepareIntent,
        source: QueuePreviewPrepareSource,
        #[cfg(test)] fault: Fault,
    ) -> Result<QueuePreviewPrepared, SqlImmediateFailure> {
        self.prepare_queue_preview_attempt(
            command,
            source,
            None,
            #[cfg(test)]
            fault,
        )
        .map_err(|flow| match flow {
            Flow::Fail(failure) => failure.error,
            Flow::Need(_) => unreachable!("static queue preparation cannot pause for Native"),
        })
    }
    pub(crate) fn prepare_queue_preview_context(
        &self,
        command: &QueuePreviewPrepareIntent,
        source: QueuePreviewPrepareSource,
        writing: &mut dyn WritingContextReader,
    ) -> writing_source::SourceResult<QueuePreviewPrepared> {
        self.prepare_queue_preview_attempt(
            command,
            source,
            Some(writing),
            #[cfg(test)]
            Fault::None,
        )
    }
    fn prepare_queue_preview_attempt(
        &self,
        command: &QueuePreviewPrepareIntent,
        source: QueuePreviewPrepareSource,
        writing: Option<&mut dyn WritingContextReader>,
        #[cfg(test)] fault: Fault,
    ) -> writing_source::SourceResult<QueuePreviewPrepared> {
        let identity = identity(command)?;
        let mut result = None;
        let mut pause =
            writing_source::SourcePause::new(command.request_id, "queue-preview-source-pause");
        let executed = self.execute_sql(
            command.request_id,
            |tx, fence| {
                receipts::check_state(tx, command.request_id, fence, "queue-prepare")?;
                receipts::check_cross_kind(tx, command.request_id, fence, "queue-prepare")?;
                let receipt = receipts::lookup(tx, &identity, fence, "queue-prepare")?;
                if let Some((state, receipt)) = replay(tx, command, fence, receipt)? {
                    result = Some(state);
                    return Ok(receipt);
                }
                let prepared = match writing {
                    Some(writing) => {
                        queue::prepare_with_context_in(tx, command, source, Some(writing))
                    }
                    None => queue::prepare_in(tx, command, source).map_err(Flow::original),
                }
                .map_err(|flow| {
                    flow.map_error(|cause| {
                        materialization_error::materialization_error(command, fence, cause)
                    })
                });
                result = Some(pause.capture(prepared, fence)?);
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        );
        pause.finish(
            executed.map(|_| result.expect("successful prepare transaction sets its result")),
        )
    }
}

#[cfg(test)]
#[path = "preview_prepare/tests.rs"]
mod tests;
