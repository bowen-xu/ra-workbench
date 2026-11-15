use super::*;

impl Database {
    /// Pure inspection of the original proof/result. No queue edit, provenance
    /// restoration, materialization, permission renewal or owner repair occurs.
    pub(crate) fn input_queue_edit_status(
        &self,
        intent: &QueueEditIntent,
    ) -> Result<QueueEditStatus, QueueEditFailure> {
        let identity = intent.identity()?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(intent.request_id))?;
        let guard = owner
            .inspection_guard()
            .map_err(|cause| admission_error(intent.request_id, cause, None))?;
        let snapshot = guard.snapshot();
        let fence = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        if intent
            .expected_epoch
            .is_some_and(|epoch| epoch != fence.epoch)
        {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                intent.request_id,
                fence,
                "queue-edit-status-epoch",
                "data_epoch_changed",
            )))
            .into());
        }
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                intent.request_id,
                fence,
                "queue-edit-status",
                "sql_mutex_poisoned",
            )))
        })?;
        let unconfirmed = |fence, reason| {
            QueueEditStatus::Unconfirmed(Box::new(diagnostic(
                intent.request_id,
                fence,
                "queue-edit-status",
                reason,
            )))
        };
        if !db.is_autocommit() {
            return Ok(unconfirmed(fence, "sql_transaction_open"));
        }
        let fence =
            receipts::status_fence(&db, intent.request_id, fence, snapshot.accepted_position)?;
        receipts::check_cross_kind(&db, intent.request_id, fence, "queue-edit-status")?;
        if let Some(receipt) = receipts::lookup(&db, &identity, fence, "queue-edit-status")? {
            return match frozen(&db, intent, fence)? {
                Some(queue) => Ok(QueueEditStatus::Committed { queue, receipt }),
                None if deleted(&db, intent.conversation_id).map_err(|cause| {
                    receipts::storage(intent.request_id, fence, "queue-edit-status", &cause)
                })? =>
                {
                    let detail = diagnostic(
                        intent.request_id,
                        receipt.sql_fence,
                        "queue-edit-status",
                        "queue_edit_result_deleted",
                    );
                    Ok(QueueEditStatus::Deleted {
                        receipt,
                        detail: Box::new(detail),
                    })
                }
                None => Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                    intent.request_id,
                    fence,
                    "queue-edit-status",
                    "queue_edit_business_receipt_missing",
                )))
                .into()),
            };
        }
        // A legacy business row lacks full original host proof. Check its exact
        // original command using only its frozen result, never Restore sources.
        let mut control = None;
        let legacy = raw(
            input_queue::read_legacy_edit_sql(
                &db,
                intent.conversation_id,
                intent.request_id,
                intent.expected_revision,
                intent.lowered(),
            ),
            intent,
            fence,
            "queue-edit-status",
            &mut control,
        )
        .map_err(|cause| match control {
            Some(Control::Domain(error)) => QueueEditFailure::Domain(error),
            _ => cause.into(),
        })?
        .is_some();
        if snapshot.blocked {
            Ok(unconfirmed(fence, "owner_blocked_receipt_unconfirmed"))
        } else if snapshot.closed {
            Ok(unconfirmed(fence, "owner_closed_receipt_unconfirmed"))
        } else if legacy {
            Ok(unconfirmed(
                fence,
                "legacy_queue_edit_receipt_without_sql_proof",
            ))
        } else if snapshot.accepted_position > fence.position {
            Ok(unconfirmed(fence, "pending_suffix_receipt_unconfirmed"))
        } else {
            Ok(QueueEditStatus::NotFound)
        }
    }
}
