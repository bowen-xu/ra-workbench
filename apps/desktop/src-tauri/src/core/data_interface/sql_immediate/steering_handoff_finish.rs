//! Atomically complete a proven queue-to-steering transfer.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, Transaction, diagnostic, queue_edit, receipts, steering_handoff,
};
use crate::agent::repository::{SqlRepositoryError, input_queue, steering};
use research_agent_core::{
    input_queue::QueueEdit,
    steering::{SteeringInput, SteeringScope},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FAMILY: &str = "agent.steering.handoff.finish";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FinishSteeringHandoff {
    pub intent: steering_handoff::SteeringHandoffIntent,
    pub expected_epoch: Uuid,
}

impl FinishSteeringHandoff {
    pub(crate) fn operation_id(&self) -> Result<Uuid, SqlImmediateFailure> {
        Ok(self.identity()?.operation_id)
    }
    pub(super) fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        self.intent.identity()?;
        if self.expected_epoch.is_nil() {
            return Err(SqlImmediateFailure::Invalid(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    self.intent.operation_id,
                    None,
                    "steering-handoff-finish",
                    "invalid_handoff_epoch",
                ),
            )));
        }
        // Derive a stable, independently namespaced UUID so a restarted client
        // can reconstruct the same exact operation without a second outbox.
        let mut id_bytes = [0u8; 16];
        id_bytes.copy_from_slice(
            &Sha256::digest([FAMILY.as_bytes(), self.intent.operation_id.as_bytes()].concat())
                [..16],
        );
        id_bytes[6] = (id_bytes[6] & 0x0f) | 0x80;
        id_bytes[8] = (id_bytes[8] & 0x3f) | 0x80;
        let bytes = serde_json::to_vec(&(FAMILY, 1u32, self)).map_err(|_| {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                self.intent.operation_id,
                None,
                "steering-handoff-finish",
                "handoff_encoding_failed",
            )))
        })?;
        Ok(ReceiptIdentity {
            operation_id: Uuid::from_bytes(id_bytes),
            kind: FAMILY,
            version: 1,
            fingerprint: Sha256::digest(bytes).into(),
        })
    }
}

impl Database {
    pub(crate) fn finish_steering_handoff(
        &self,
        request: &FinishSteeringHandoff,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let identity = request.identity()?;
        self.execute_sql(
            identity.operation_id,
            |tx, fence| finish(tx, fence, request, &identity),
            #[cfg(test)]
            super::Fault::None,
        )
    }
}

fn finish(
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
            "steering-handoff-finish",
            reason,
        )))
    };
    if request.expected_epoch != fence.epoch {
        return Err(conflict("data_epoch_changed"));
    }
    let intent = &request.intent;
    receipts::check_state(tx, operation, fence, "steering-handoff-finish")?;
    receipts::check_cross_kind(tx, operation, fence, "steering-handoff-finish")?;
    let queue_intent = queue_edit::QueueEditIntent {
        conversation_id: intent.conversation_id,
        request_id: operation,
        expected_revision: intent.expected_revision,
        expected_epoch: Some(request.expected_epoch),
        scope: intent.scope.clone(),
        edit: QueueEdit::Remove(intent.queue_input_id),
    };
    queue_edit::authorization::authorize(tx, &queue_intent).map_err(|error| match error {
        SqlRepositoryError::Sql(error) => {
            receipts::storage(operation, fence, "steering-handoff-authorize", &error)
        }
        SqlRepositoryError::Domain(_) => conflict("handoff_scope_unavailable"),
    })?;
    if let Some(receipt) = receipts::lookup(tx, identity, fence, "steering-handoff-finish")? {
        return Ok(receipt);
    }
    let original = intent.identity()?;
    receipts::check_cross_kind(tx, original.operation_id, fence, "steering-handoff-finish")?;
    if receipts::lookup(tx, &original, fence, "steering-handoff-finish")?.is_none() {
        return Err(conflict("handoff_prepare_unconfirmed"));
    }
    let saved: Option<(String, String)> = tx
        .query_row(
            "select epoch,intent_json from agent_steering_handoffs where steering_input_id=?1",
            [intent.steering_input_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| receipts::storage(operation, fence, "steering-handoff-finish", &error))?;
    let encoded = serde_json::to_string(intent).map_err(|_| conflict("handoff_encoding_failed"))?;
    if saved != Some((fence.epoch.to_string(), encoded)) {
        return Err(conflict("handoff_identity_changed"));
    }
    let input = SteeringInput {
        scope: SteeringScope {
            conversation_id: intent.conversation_id,
            turn_id: intent.turn_id,
        },
        input_id: intent.steering_input_id,
        text: intent.text.clone(),
        user_input: intent.user_input.clone(),
        skill_instructions: vec![],
    };
    match steering_handoff::history(tx, &input) {
        Ok(steering_handoff::SteeringHandoffHistory::Confirmed) => {}
        Ok(_) => return Err(conflict("handoff_history_unconfirmed")),
        Err(_) => {
            return Err(SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-history",
                "handoff_history_unavailable",
            ))));
        }
    }
    let admitted: Option<(String, String)> = tx
        .query_row(
            "select turn_id,text from agent_steering_inputs where input_id=?1",
            [intent.steering_input_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| {
            receipts::storage(operation, fence, "steering-handoff-admission", &error)
        })?;
    if admitted != Some((intent.turn_id.to_string(), intent.text.clone())) {
        return Err(conflict("handoff_admission_changed"));
    }
    let resources =
        steering::resources::read(tx, intent.steering_input_id).map_err(|error| match error {
            research_agent_core::steering::SteeringError::Invalid => {
                conflict("handoff_admission_invalid")
            }
            _ => SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-admission",
                "handoff_admission_unavailable",
            ))),
        })?;
    if resources.0 != intent.user_input {
        return Err(conflict("handoff_admission_changed"));
    }
    let queue = input_queue::load_sql(tx, intent.conversation_id).map_err(|error| match error {
        SqlRepositoryError::Sql(error) => {
            receipts::storage(operation, fence, "steering-handoff-queue", &error)
        }
        SqlRepositoryError::Domain(input_queue::Error::Corrupt) => {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-queue",
                "handoff_queue_state_corrupt",
            )))
        }
        SqlRepositoryError::Domain(input_queue::Error::Storage) => {
            SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-queue",
                "handoff_queue_storage_unavailable",
            )))
        }
        SqlRepositoryError::Domain(_) => conflict("handoff_queue_unavailable"),
    })?;
    if queue.preparing().is_some()
        || !queue.inputs().iter().any(|entry| {
            entry.id == intent.queue_input_id
                && entry.text == intent.text
                && entry.user_input == intent.user_input
                && entry.origin == intent.origin
        })
    {
        return Err(conflict("handoff_queue_changed"));
    }
    let existing = input_queue::read_edit_receipt_sql(tx, intent.conversation_id, operation)
        .map_err(|error| match error {
            SqlRepositoryError::Sql(error) => {
                receipts::storage(operation, fence, "steering-handoff-receipt", &error)
            }
            SqlRepositoryError::Domain(input_queue::Error::Corrupt) => {
                SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    operation,
                    fence,
                    "steering-handoff-receipt",
                    "handoff_queue_receipt_corrupt",
                )))
            }
            SqlRepositoryError::Domain(input_queue::Error::Storage) => {
                SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                    operation,
                    fence,
                    "steering-handoff-receipt",
                    "handoff_queue_receipt_unavailable",
                )))
            }
            SqlRepositoryError::Domain(_) => conflict("handoff_operation_already_used"),
        })?;
    if existing.is_some() {
        return Err(conflict("handoff_operation_already_used"));
    }
    input_queue::apply_handoff_removal_in_tx_sql(
        tx,
        intent.conversation_id,
        operation,
        queue.revision(),
        intent.queue_input_id,
    )
    .map_err(|error| match error {
        SqlRepositoryError::Sql(error) => {
            receipts::storage(operation, fence, "steering-handoff-remove", &error)
        }
        SqlRepositoryError::Domain(input_queue::Error::Corrupt) => {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-remove",
                "handoff_queue_state_corrupt",
            )))
        }
        SqlRepositoryError::Domain(input_queue::Error::Storage) => {
            SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-remove",
                "handoff_queue_storage_unavailable",
            )))
        }
        SqlRepositoryError::Domain(_) => conflict("handoff_queue_changed"),
    })?;
    let removed = tx
        .execute(
            "delete from agent_steering_handoffs where steering_input_id=?1",
            params![intent.steering_input_id.to_string()],
        )
        .map_err(|error| receipts::storage(operation, fence, "steering-handoff-finish", &error))?;
    if removed != 1 {
        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
            operation,
            fence,
            "steering-handoff-finish",
            "handoff_removal_unconfirmed",
        ))));
    }
    receipts::insert(tx, identity, fence)
}
