use super::*;
impl Database {
    pub(crate) fn writing_prepare_status(
        &self,
        command: &WritingPrepareIntent,
    ) -> Result<WritingPrepareStatus, SqlImmediateFailure> {
        let identity = identity(command)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.operation()))?;
        let guard = owner
            .inspection_guard()
            .map_err(|cause| admission_error(command.operation(), cause, None))?;
        let snapshot = guard.snapshot();
        let cached = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            failure(
                command,
                cached,
                "writing-prepare-status",
                "sql_mutex_poisoned",
            )
        })?;
        let uncertain = |fence, cause| {
            WritingPrepareStatus::Unconfirmed(Box::new(diagnostic(
                command.operation(),
                fence,
                "writing-prepare-status",
                cause,
            )))
        };
        if !db.is_autocommit() {
            return Ok(uncertain(cached, "sql_transaction_open"));
        }
        let fence =
            receipts::status_fence(&db, command.operation(), cached, snapshot.accepted_position)?;
        receipts::check_cross_kind(&db, command.operation(), fence, "writing-prepare-status")?;
        if let Some(receipt) = receipts::lookup(&db, &identity, fence, "writing-prepare-status")? {
            return Ok(match committed_in(&db, command, fence)? {
                PreparedFact::Present(record) => WritingPrepareStatus::Committed {
                    record,
                    receipt: Box::new(receipt),
                },
                PreparedFact::Deleted => {
                    uncertain(receipt.sql_fence, "writing_prepare_result_deleted")
                }
            });
        }
        if snapshot.accepted_position > fence.position {
            return Ok(uncertain(
                fence,
                "writing_prepare_pending_prefix_unconfirmed",
            ));
        }
        if snapshot.blocked {
            return Ok(uncertain(fence, "owner_blocked_receipt_unconfirmed"));
        }
        if snapshot.closed {
            return Ok(uncertain(fence, "owner_closed_receipt_unconfirmed"));
        }
        if record_in(&db, command, fence)?.is_some() {
            return Ok(uncertain(fence, "legacy_prepare_without_intent_proof"));
        }
        Ok(WritingPrepareStatus::NotFound)
    }
}
