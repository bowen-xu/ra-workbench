//! Archive/restore shares the journal's admission and the existing SQL receipt.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, SqlImmediateStatus, SqlMutationError, Transaction, receipts,
};
use crate::agent::repository::ValidatedArchive;
use research_agent_core::conversation::ArchiveConversation;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const KIND: &str = "agent.conversation.archive";
const VERSION: u32 = 1;

fn identity(command: &ArchiveConversation) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    command.validate().map_err(SqlImmediateFailure::Domain)?;
    #[derive(Serialize)]
    struct Frozen {
        kind: &'static str,
        version: u32,
        operation_id: Uuid,
        conversation_id: Uuid,
        archived: bool,
        expected_watermark: Option<u64>,
    }
    let bytes = serde_json::to_vec(&Frozen {
        kind: KIND,
        version: VERSION,
        operation_id: command.operation_id,
        conversation_id: *command.id.as_uuid(),
        archived: command.archived,
        expected_watermark: command.expected_watermark,
    })
    .map_err(|_| {
        SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
            command.operation_id,
            None,
            "validate",
            "archive_encoding_failed",
        )))
    })?;
    Ok(ReceiptIdentity {
        operation_id: command.operation_id,
        kind: KIND,
        version: VERSION,
        fingerprint: Sha256::digest(bytes).into(),
    })
}

impl Database {
    /// A write-edge replay probe, never write authorization from inspection.
    /// Missing receipts do not touch a clock, business row or receipt.
    pub(crate) fn archive_replay(
        &self,
        command: &ArchiveConversation,
    ) -> Result<Option<SqlImmediateReceipt>, SqlImmediateFailure> {
        let identity = identity(command)?;
        let operation = command.operation_id;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| super::unmanaged(operation))?;
        let mut admission = owner.materialize_guard().map_err(|error| {
            let fence = owner.inspection_guard().ok().map(|guard| {
                let state = guard.snapshot();
                ReadFence {
                    epoch: state.epoch,
                    position: state.materialized_position,
                }
            });
            super::admission_error(operation, error, fence)
        })?;
        let fence = admission.fence();
        let db = self.raw.lock().map_err(|_| {
            admission.block();
            SqlImmediateFailure::RecoveryRequired(Box::new(super::diagnostic(
                operation,
                fence,
                "archive-replay-lock",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            admission.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(
                super::diagnostic(
                    operation,
                    fence,
                    "archive-replay",
                    "sql_transaction_already_open",
                ),
            )));
        }
        let result = (|| {
            receipts::check_state(&db, operation, fence, "archive-replay")?;
            receipts::check_cross_kind(&db, operation, fence, "archive-replay")?;
            receipts::lookup(&db, &identity, fence, "archive-replay")
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            admission.block();
        }
        result
    }

    pub(crate) fn archive_conversation(
        &self,
        prepared: &ValidatedArchive,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        self.archive_conversation_owned(
            prepared,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    fn archive_conversation_owned(
        &self,
        prepared: &ValidatedArchive,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let command = prepared.command();
        let identity = identity(command)?;
        self.execute_sql(
            command.operation_id,
            |tx, fence| register(tx, command, &identity, fence),
            #[cfg(test)]
            fault,
        )
    }

    pub(crate) fn archive_status(
        &self,
        command: &ArchiveConversation,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        self.sql_operation_status(&identity(command)?)
    }
}

fn register(
    tx: &Transaction<'_>,
    command: &ArchiveConversation,
    identity: &ReceiptIdentity,
    fence: ReadFence,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation = identity.operation_id;
    receipts::check_state(tx, operation, fence, "prepare")?;
    receipts::check_cross_kind(tx, operation, fence, "prepare")?;
    if let Some(receipt) = receipts::lookup(tx, identity, fence, "prepare")? {
        return Ok(receipt);
    }
    crate::agent::repository::check_archive_in(tx, command)
        .map_err(|error| mutation_error(operation, fence, "validate", error))?;
    let updated_at: String = tx
        .query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(|error| receipts::storage(operation, fence, "prepare", &error))?;
    crate::agent::repository::archive_in(tx, command, &updated_at)
        .map_err(|error| mutation_error(operation, fence, "mutate", error))?;
    receipts::insert(tx, identity, fence)
}

fn mutation_error(
    operation: Uuid,
    fence: ReadFence,
    phase: &'static str,
    error: SqlMutationError,
) -> SqlImmediateFailure {
    match error {
        SqlMutationError::Domain(error) => SqlImmediateFailure::Domain(error),
        SqlMutationError::Sql(error) => receipts::storage(operation, fence, phase, &error),
    }
}

#[cfg(test)]
#[path = "archive/tests.rs"]
mod tests;
