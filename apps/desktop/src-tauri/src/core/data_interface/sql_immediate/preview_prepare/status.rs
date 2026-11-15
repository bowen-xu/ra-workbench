use super::*;

impl Database {
    /// Inspection reads proof and business result under one owner/SQL lock.
    /// It never materializes a suffix, repairs an owner or bridges legacy data.
    pub(crate) fn read_queue_preview_prepare_status(
        &self,
        command: &QueuePreviewPrepareIntent,
    ) -> Result<QueuePreviewPrepareStatus, SqlImmediateFailure> {
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
                "queue-prepare-status",
                "sql_mutex_poisoned",
            )))
        })?;
        let unconfirmed = |fence, reason| {
            QueuePreviewPrepareStatus::Unconfirmed(Box::new(diagnostic(
                command.request_id,
                fence,
                "queue-prepare-status",
                reason,
            )))
        };
        if !db.is_autocommit() {
            return Ok(unconfirmed(fence, "sql_transaction_open"));
        }
        let fence =
            receipts::status_fence(&db, command.request_id, fence, snapshot.accepted_position)?;
        receipts::check_cross_kind(&db, command.request_id, fence, "queue-prepare-status")?;
        let receipt = receipts::lookup(&db, &identity, fence, "queue-prepare-status")?;
        let saved = queue::read_in(&db, command)
            .map_err(|cause| error(command, fence, "queue-prepare-status", cause))?;
        match (receipt, saved) {
            (Some(receipt), Preparation::Prepared(state)) => {
                Ok(QueuePreviewPrepareStatus::Committed {
                    state,
                    receipt: Box::new(receipt),
                })
            }
            (Some(_), _) => Err(corrupt(
                command,
                fence,
                "queue-prepare-status",
                "queue_prepare_business_receipt_missing",
            )),
            (None, _) if snapshot.blocked => {
                Ok(unconfirmed(fence, "owner_blocked_receipt_unconfirmed"))
            }
            (None, _) if snapshot.closed => {
                Ok(unconfirmed(fence, "owner_closed_receipt_unconfirmed"))
            }
            (None, Preparation::Legacy) => {
                Ok(unconfirmed(fence, "legacy_prepare_without_intent_proof"))
            }
            (None, Preparation::Prepared(_)) => Ok(unconfirmed(
                fence,
                "prepare_business_receipt_without_sql_proof",
            )),
            (None, Preparation::Missing) => Ok(QueuePreviewPrepareStatus::NotFound),
        }
    }
}
