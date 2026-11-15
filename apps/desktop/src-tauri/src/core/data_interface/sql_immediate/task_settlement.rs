//! Proof that the original submission owner finished its cleanup successfully.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, Transaction, receipts,
};
use crate::agent::repository::{
    SqlRepositoryError,
    submissions::{self, start_observation},
};
use research_agent_core::protocol::TaskKind;
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const KIND: &str = "agent.task.owner_settled";
const RUN_KIND: &str = "agent.task.run_settled";

fn run_identity(
    conversation_id: Uuid,
    turn_id: Uuid,
    run_id: Uuid,
) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    let mut name = b"agent.task.run_settled:v1:".to_vec();
    name.extend_from_slice(turn_id.as_bytes());
    let operation_id = Uuid::new_v5(&conversation_id, &name);
    if conversation_id.is_nil() || turn_id.is_nil() || run_id.is_nil() {
        return Err(SqlImmediateFailure::Invalid(Box::new(
            SqlImmediateDiagnostic::for_operation(
                operation_id,
                None,
                "task-run-settlement",
                "invalid_task_identity",
            ),
        )));
    }
    let encoded = serde_json::to_vec(&(RUN_KIND, 1_u32, conversation_id, turn_id, run_id))
        .map_err(|_| {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                operation_id,
                None,
                "task-run-settlement",
                "task_identity_encoding_failed",
            )))
        })?;
    Ok(ReceiptIdentity {
        operation_id,
        kind: RUN_KIND,
        version: 1,
        fingerprint: Sha256::digest(encoded).into(),
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskSettlement {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub turn_id: Uuid,
}
impl TaskSettlement {
    fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        let mut name = b"agent.task.owner_settled:v1:".to_vec();
        name.extend_from_slice(self.request_id.as_bytes());
        name.extend_from_slice(self.turn_id.as_bytes());
        let operation_id = Uuid::new_v5(&self.conversation_id, &name);
        if self.conversation_id.is_nil() || self.request_id.is_nil() || self.turn_id.is_nil() {
            return Err(SqlImmediateFailure::Invalid(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    operation_id,
                    None,
                    "task-settlement",
                    "invalid_task_identity",
                ),
            )));
        }
        let encoded = serde_json::to_vec(&(KIND, 1_u32, self)).map_err(|_| {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                operation_id,
                None,
                "task-settlement",
                "task_identity_encoding_failed",
            )))
        })?;
        Ok(ReceiptIdentity {
            operation_id,
            kind: KIND,
            version: 1,
            fingerprint: Sha256::digest(encoded).into(),
        })
    }
}
impl Database {
    pub(crate) fn record_task_settlement(
        &self,
        task: &TaskSettlement,
        run_id: Uuid,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let identity = task.identity()?;
        let run = run_identity(task.conversation_id, task.turn_id, run_id)?;
        self.execute_sql(
            identity.operation_id,
            |tx, fence| {
                let receipt = register(tx, fence, task, &identity)?;
                receipts::check_cross_kind(tx, run.operation_id, fence, "task-run-settlement")?;
                if receipts::lookup(tx, &run, fence, "task-run-settlement")?.is_none() {
                    if receipt.replayed {
                        return Err(SqlImmediateFailure::Invalid(Box::new(
                            SqlImmediateDiagnostic::for_operation(
                                run.operation_id,
                                Some(fence),
                                "task-run-settlement",
                                "original_run_identity_unavailable",
                            ),
                        )));
                    }
                    receipts::insert(tx, &run, fence)?;
                }
                Ok(receipt)
            },
            #[cfg(test)]
            super::Fault::None,
        )
    }
    pub(crate) fn inspect_task_run_settlement(
        &self,
        conversation_id: Uuid,
        turn_id: Uuid,
        run_id: Uuid,
    ) -> Result<bool, SqlImmediateFailure> {
        let identity = run_identity(conversation_id, turn_id, run_id)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| super::unmanaged(identity.operation_id))?;
        let guard = owner
            .inspection_guard()
            .map_err(|error| super::admission_error(identity.operation_id, error, None))?;
        let snapshot = guard.snapshot();
        let cached = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(super::diagnostic(
                identity.operation_id,
                cached,
                "task-run-settlement-status",
                "sql_mutex_poisoned",
            )))
        })?;
        if snapshot.blocked || snapshot.closed || !db.is_autocommit() {
            return Ok(false);
        }
        let fence = receipts::status_fence(
            &db,
            identity.operation_id,
            cached,
            snapshot.accepted_position,
        )?;
        if fence.position != snapshot.accepted_position {
            return Ok(false);
        }
        receipts::check_cross_kind(
            &db,
            identity.operation_id,
            fence,
            "task-run-settlement-status",
        )?;
        match receipts::lookup(&db, &identity, fence, "task-run-settlement-status") {
            Ok(receipt) => Ok(receipt.is_some()),
            Err(SqlImmediateFailure::Conflict(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }
    #[cfg(test)]
    pub(crate) fn task_settlement_status(
        &self,
        task: &TaskSettlement,
    ) -> Result<super::SqlImmediateStatus, SqlImmediateFailure> {
        self.sql_operation_status(&task.identity()?)
    }
}

fn register(
    tx: &Transaction<'_>,
    fence: ReadFence,
    task: &TaskSettlement,
    identity: &ReceiptIdentity,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation = identity.operation_id;
    receipts::check_state(tx, operation, fence, "task-settlement")?;
    receipts::check_cross_kind(tx, operation, fence, "task-settlement")?;
    let invalid = || {
        SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
            operation,
            Some(fence),
            "task-settlement",
            "task_history_not_settled",
        )))
    };
    let record = submissions::read_sql(tx, task.conversation_id, task.request_id)
        .map_err(|error| match error {
            SqlRepositoryError::Sql(error) => {
                receipts::storage(operation, fence, "task-settlement-read", &error)
            }
            SqlRepositoryError::Domain(_) => invalid(),
        })?
        .ok_or_else(invalid)?;
    if record.turn_id != task.turn_id {
        return Err(invalid());
    }
    let observation = start_observation::inspect(tx, &record, TaskKind::Regular)
        .map_err(|error| match error {
            start_observation::Error::Unavailable(error)
            | start_observation::Error::InvalidRow(error) => {
                receipts::storage(operation, fence, "task-settlement-history", &error)
            }
            _ => invalid(),
        })?
        .ok_or_else(invalid)?;
    if observation.outcome.is_none() || observation.unresolved_tool_effect {
        return Err(invalid());
    }
    if let Some(receipt) = receipts::lookup(tx, identity, fence, "task-settlement")? {
        return Ok(receipt);
    }
    receipts::insert(tx, identity, fence)
}

pub(super) fn settled(
    db: &Connection,
    fence: ReadFence,
    task: &TaskSettlement,
) -> Result<bool, SqlImmediateFailure> {
    let identity = task.identity()?;
    receipts::check_cross_kind(db, identity.operation_id, fence, "task-settlement-status")?;
    Ok(receipts::lookup(db, &identity, fence, "task-settlement-status")?.is_some())
}
