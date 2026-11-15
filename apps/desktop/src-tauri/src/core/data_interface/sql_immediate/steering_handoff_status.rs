//! Inspect immutable resolution receipts without performing a resolution.
use super::{
    Database, ReadFence, SqlImmediateFailure, SqlImmediateReceipt, admission_error, diagnostic,
    queue_edit, receipts, steering_handoff_cancel, steering_handoff_finish::FinishSteeringHandoff,
    steering_handoff_release, unmanaged,
};
use crate::agent::repository::SqlRepositoryError;
use research_agent_core::input_queue::QueueEdit;

pub(crate) enum SteeringHandoffResolution {
    Unconfirmed,
    Finished(SqlImmediateReceipt),
    Released(SqlImmediateReceipt),
    Canceled(SqlImmediateReceipt),
}

impl Database {
    pub(crate) fn inspect_steering_handoff_resolution(
        &self,
        request: &FinishSteeringHandoff,
    ) -> Result<SteeringHandoffResolution, SqlImmediateFailure> {
        let original = request.intent.identity()?;
        let identities = [
            request.identity()?,
            steering_handoff_release::identity(request)?,
            steering_handoff_cancel::identity(request)?,
        ];
        let operation = original.operation_id;
        let owner = self.owner.as_ref().ok_or_else(|| unmanaged(operation))?;
        let guard = owner
            .inspection_guard()
            .map_err(|error| admission_error(operation, error, None))?;
        let snapshot = guard.snapshot();
        let cached = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        if request.expected_epoch != cached.epoch
            || request
                .intent
                .expected_epoch
                .is_some_and(|epoch| epoch != cached.epoch)
        {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                operation,
                cached,
                "steering-handoff-resolution",
                "data_epoch_changed",
            ))));
        }
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                operation,
                cached,
                "steering-handoff-resolution",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            return Ok(SteeringHandoffResolution::Unconfirmed);
        }
        let tx = db.unchecked_transaction().map_err(|error| {
            receipts::storage(operation, cached, "steering-handoff-resolution", &error)
        })?;
        let fence = receipts::status_fence(&tx, operation, cached, snapshot.accepted_position)?;
        let scope = queue_edit::QueueEditIntent {
            conversation_id: request.intent.conversation_id,
            request_id: operation,
            expected_revision: request.intent.expected_revision,
            expected_epoch: Some(request.expected_epoch),
            scope: request.intent.scope.clone(),
            edit: QueueEdit::Remove(request.intent.queue_input_id),
        };
        queue_edit::authorization::authorize(&tx, &scope).map_err(|error| match error {
            SqlRepositoryError::Sql(error) => {
                receipts::storage(operation, fence, "steering-handoff-resolution", &error)
            }
            SqlRepositoryError::Domain(_) => SqlImmediateFailure::Conflict(Box::new(diagnostic(
                operation,
                fence,
                "steering-handoff-resolution",
                "handoff_scope_unavailable",
            ))),
        })?;
        receipts::check_cross_kind(&tx, operation, fence, "steering-handoff-resolution")?;
        let prepared =
            receipts::lookup(&tx, &original, fence, "steering-handoff-resolution")?.is_some();
        let mut result = None;
        for (index, identity) in identities.iter().enumerate() {
            receipts::check_cross_kind(
                &tx,
                identity.operation_id,
                fence,
                "steering-handoff-resolution",
            )?;
            if let Some(receipt) =
                receipts::lookup(&tx, identity, fence, "steering-handoff-resolution")?
            {
                if result.is_some() || (index < 2) != prepared {
                    return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                        operation,
                        fence,
                        "steering-handoff-resolution",
                        "handoff_resolution_inconsistent",
                    ))));
                }
                result = Some(match index {
                    0 => SteeringHandoffResolution::Finished(receipt),
                    1 => SteeringHandoffResolution::Released(receipt),
                    _ => SteeringHandoffResolution::Canceled(receipt),
                });
            }
        }
        if result.is_some() {
            let present: bool = tx.query_row(
                "select exists(select 1 from agent_steering_handoffs where steering_input_id=?1 or operation_id=?2)",
                rusqlite::params![request.intent.steering_input_id.to_string(), operation.to_string()],
                |row| row.get(0),
            ).map_err(|error| receipts::storage(operation, fence, "steering-handoff-resolution", &error))?;
            if present {
                return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    operation,
                    fence,
                    "steering-handoff-resolution",
                    "handoff_resolution_inconsistent",
                ))));
            }
        }
        Ok(result.unwrap_or(SteeringHandoffResolution::Unconfirmed))
    }
}
