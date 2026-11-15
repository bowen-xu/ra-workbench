//! Fence a never-registered handoff against a delayed original request.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, Transaction, diagnostic, queue_edit, receipts,
    steering_handoff_finish::FinishSteeringHandoff,
};
use crate::agent::repository::SqlRepositoryError;
use research_agent_core::input_queue::QueueEdit;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FAMILY: &str = "agent.steering.handoff.cancel";
pub(super) fn identity(
    request: &FinishSteeringHandoff,
) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    request.intent.identity()?;
    if request.expected_epoch.is_nil() {
        return Err(SqlImmediateFailure::Invalid(Box::new(
            SqlImmediateDiagnostic::for_operation(
                request.intent.operation_id,
                None,
                "steering-handoff-cancel",
                "invalid_handoff_epoch",
            ),
        )));
    }
    let mut id = [0u8; 16];
    id.copy_from_slice(
        &Sha256::digest([FAMILY.as_bytes(), request.intent.operation_id.as_bytes()].concat())[..16],
    );
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    let bytes = serde_json::to_vec(&(FAMILY, 1u32, request)).map_err(|_| {
        SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
            request.intent.operation_id,
            None,
            "steering-handoff-cancel",
            "handoff_encoding_failed",
        )))
    })?;
    Ok(ReceiptIdentity {
        operation_id: Uuid::from_bytes(id),
        kind: FAMILY,
        version: 1,
        fingerprint: Sha256::digest(bytes).into(),
    })
}
impl FinishSteeringHandoff {
    pub(crate) fn cancel_operation_id(&self) -> Result<Uuid, SqlImmediateFailure> {
        Ok(identity(self)?.operation_id)
    }
}
impl Database {
    pub(crate) fn cancel_unregistered_steering_handoff(
        &self,
        request: &FinishSteeringHandoff,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let identity = identity(request)?;
        self.execute_sql(
            identity.operation_id,
            |tx, fence| cancel(tx, fence, request, &identity),
            #[cfg(test)]
            super::Fault::None,
        )
    }
}
fn cancel(
    tx: &Transaction<'_>,
    fence: ReadFence,
    request: &FinishSteeringHandoff,
    identity: &ReceiptIdentity,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation = identity.operation_id;
    let conflict = |reason| {
        SqlImmediateFailure::Conflict(Box::new(diagnostic(
            operation,
            fence,
            "steering-handoff-cancel",
            reason,
        )))
    };
    if request.expected_epoch != fence.epoch
        || request
            .intent
            .expected_epoch
            .is_some_and(|epoch| epoch != request.expected_epoch)
    {
        return Err(conflict("data_epoch_changed"));
    }
    let intent = &request.intent;
    receipts::check_state(tx, operation, fence, "steering-handoff-cancel")?;
    receipts::check_cross_kind(tx, operation, fence, "steering-handoff-cancel")?;
    let scope = queue_edit::QueueEditIntent {
        conversation_id: intent.conversation_id,
        request_id: operation,
        expected_revision: intent.expected_revision,
        expected_epoch: Some(request.expected_epoch),
        scope: intent.scope.clone(),
        edit: QueueEdit::Remove(intent.queue_input_id),
    };
    queue_edit::authorization::authorize(tx, &scope).map_err(|error| match error {
        SqlRepositoryError::Sql(error) => {
            receipts::storage(operation, fence, "steering-handoff-authorize", &error)
        }
        SqlRepositoryError::Domain(_) => conflict("handoff_scope_unavailable"),
    })?;
    if let Some(receipt) = receipts::lookup(tx, identity, fence, "steering-handoff-cancel")? {
        return Ok(receipt);
    }
    let original = intent.identity()?;
    receipts::check_cross_kind(tx, original.operation_id, fence, "steering-handoff-cancel")?;
    if receipts::lookup(tx, &original, fence, "steering-handoff-cancel")?.is_some() {
        return Err(conflict("handoff_already_registered"));
    }
    let saved:Option<String>=tx.query_row("select operation_id from agent_steering_handoffs where steering_input_id=?1 or operation_id=?2 limit 1",
        rusqlite::params![intent.steering_input_id.to_string(),intent.operation_id.to_string()],|row|row.get(0)).optional()
        .map_err(|error|receipts::storage(operation,fence,"steering-handoff-cancel",&error))?;
    if let Some(saved) = saved {
        if saved != intent.operation_id.to_string() {
            return Err(conflict("handoff_identity_in_use"));
        }
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            operation,
            fence,
            "steering-handoff-cancel",
            "handoff_receipt_mismatch",
        ))));
    }
    receipts::insert(tx, identity, fence)
}
