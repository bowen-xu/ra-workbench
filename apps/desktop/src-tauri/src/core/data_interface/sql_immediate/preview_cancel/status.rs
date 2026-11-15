use super::*;

impl Database {
    /// Inspection never projects logs, writes a proof or repairs an uncertain
    /// owner. A legacy business row alone is deliberately not Committed.
    pub(crate) fn queue_preview_cancel_status(
        &self,
        command: &CancelPreviewIntent,
    ) -> Result<QueuePreviewCancelStatus, SqlImmediateFailure> {
        let identity = identity(command)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.request_id))?;
        let guard = owner
            .inspection_guard()
            .map_err(|cause| admission_error(command.request_id, cause, None))?;
        let snapshot = guard.snapshot();
        let fence = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-cancel-status",
                "sql_mutex_poisoned",
            )))
        })?;
        let unconfirmed = |fence, reason| {
            QueuePreviewCancelStatus::Unconfirmed(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-cancel-status",
                reason,
            )))
        };
        if !db.is_autocommit() {
            return Ok(unconfirmed(fence, "sql_transaction_open"));
        }
        let fence =
            receipts::status_fence(&db, command.request_id, fence, snapshot.accepted_position)?;
        receipts::check_cross_kind(&db, command.request_id, fence, "queue-cancel-status")?;
        let receipt = receipts::lookup(&db, &identity, fence, "queue-cancel-status")?;
        let saved = business(&db, command)
            .map_err(|cause| error(command, fence, "queue-cancel-status", cause))?;
        if saved.is_some() {
            compact::check_prepare_proof(&db, command, fence)?;
        }
        if saved.is_some()
            && !frozen_scope(&db, command)
                .map_err(|cause| error(command, fence, "queue-cancel-status", cause))?
        {
            return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-cancel-status",
                "queue_cancel_frozen_scope_missing",
            ))));
        }
        match (receipt, saved) {
            (Some(receipt), Some(queue)) => Ok(QueuePreviewCancelStatus::Committed {
                queue,
                receipt: Box::new(receipt),
            }),
            (Some(_), None) => Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-cancel-status",
                "queue_cancel_business_receipt_missing",
            )))),
            (None, _) if snapshot.blocked => {
                Ok(unconfirmed(fence, "owner_blocked_receipt_unconfirmed"))
            }
            (None, _) if snapshot.closed => {
                Ok(unconfirmed(fence, "owner_closed_receipt_unconfirmed"))
            }
            (None, Some(_)) => Ok(unconfirmed(
                fence,
                "legacy_cancel_receipt_without_sql_proof",
            )),
            (None, None) => Ok(QueuePreviewCancelStatus::NotFound),
        }
    }
}
