//! Historical child, replacement contexts and the original proof publish atomically.
use super::*;
use crate::agent::repository::{PreparedFork, fork_in, validate_fork_request};
use research_agent_core::conversation::ConversationForkRequest;
use serde::Serialize;
use sha2::{Digest, Sha256};

const KIND: &str = "agent.conversation.fork";
const VERSION: u32 = 1;
fn identity(request: &ConversationForkRequest) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    validate_fork_request(request).map_err(SqlImmediateFailure::Domain)?;
    #[derive(Serialize)]
    struct Frozen<'a> {
        kind: &'static str,
        version: u32,
        child_id: Uuid,
        parent_id: Uuid,
        through_turn_id: Uuid,
        binding: &'a research_agent_core::conversation::ConversationBinding,
    }
    let operation_id = *request.child_id.as_uuid();
    let bytes = serde_json::to_vec(&Frozen {
        kind: KIND,
        version: VERSION,
        child_id: operation_id,
        parent_id: *request.parent_id.as_uuid(),
        through_turn_id: request.through_turn_id,
        binding: &request.binding,
    })
    .map_err(|_| {
        SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
            operation_id,
            None,
            "validate",
            "fork_encoding_failed",
        )))
    })?;
    Ok(ReceiptIdentity {
        operation_id,
        kind: KIND,
        version: VERSION,
        fingerprint: Sha256::digest(bytes).into(),
    })
}
impl Database {
    /// `Database` owns one immutable coordinator/SQL pair. After this read-only
    /// check, retaining this exact instance pins the epoch for the full fork.
    /// Reject before materialize_guard: a foreign owner's pending suffix must
    /// not be projected, reclaimed, or synced just to reject a stale caller.
    pub(crate) fn check_fork_epoch(
        &self,
        operation: Uuid,
        expected: Uuid,
    ) -> Result<(), SqlImmediateFailure> {
        self.check_catalog_epoch(operation, expected, "fork-epoch")
    }
    /// Write-edge lookup, never a new publication or a historical status shortcut.
    pub(crate) fn fork_replay(
        &self,
        request: &ConversationForkRequest,
    ) -> Result<Option<SqlImmediateReceipt>, SqlImmediateFailure> {
        let identity = identity(request)?;
        let operation = identity.operation_id;
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
                "fork-replay-lock",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            admission.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(
                super::diagnostic(
                    operation,
                    fence,
                    "fork-replay",
                    "sql_transaction_already_open",
                ),
            )));
        }
        let result = (|| {
            receipts::check_state(&db, operation, fence, "fork-replay")?;
            receipts::check_cross_kind(&db, operation, fence, "fork-replay")?;
            receipts::lookup(&db, &identity, fence, "fork-replay")
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            admission.block();
        }
        result
    }
    pub(crate) fn fork_conversation(
        &self,
        prepared: &PreparedFork,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        self.fork_conversation_owned(
            prepared,
            #[cfg(test)]
            super::Fault::None,
        )
    }
    fn fork_conversation_owned(
        &self,
        prepared: &PreparedFork,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let identity = identity(prepared.request())?;
        self.execute_sql(
            identity.operation_id,
            |tx, fence| {
                receipts::check_state(tx, identity.operation_id, fence, "prepare")?;
                receipts::check_cross_kind(tx, identity.operation_id, fence, "prepare")?;
                if let Some(receipt) = receipts::lookup(tx, &identity, fence, "prepare")? {
                    return Ok(receipt);
                }
                fork_in(tx, prepared).map_err(|error| match error {
                    SqlMutationError::Domain(
                        research_agent_core::conversation::CatalogError::Unavailable,
                    ) => SqlImmediateFailure::Unavailable(Box::new(super::diagnostic(
                        identity.operation_id,
                        fence,
                        "fork-publication",
                        "fork_context_unavailable",
                    ))),
                    SqlMutationError::Domain(error) => SqlImmediateFailure::Domain(error),
                    SqlMutationError::Sql(error) => {
                        receipts::storage(identity.operation_id, fence, "fork-publication", &error)
                    }
                })?;
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        )
    }
    /// Reads only the original proof, including while blocked/closed; no parent or child reads.
    pub(crate) fn fork_status(
        &self,
        request: &ConversationForkRequest,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        self.sql_operation_status(&identity(request)?)
    }
}

#[cfg(test)]
#[path = "fork/tests.rs"]
mod tests;
