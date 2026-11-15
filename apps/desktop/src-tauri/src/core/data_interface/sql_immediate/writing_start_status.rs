//! Read frozen execution facts under the existing owner, without projection work.
use super::*;
use crate::agent::repository::writing_start_inspection::{self, Fact, Intent};

pub(crate) enum WritingStartInspection {
    Confirmed(Fact),
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
impl Database {
    pub(crate) fn inspect_writing_start(&self, intent: &Intent) -> WritingStartInspection {
        let operation = intent.request_id;
        let uncertain = |fence: Option<ReadFence>, cause| {
            WritingStartInspection::Unconfirmed(Box::new(SqlImmediateDiagnostic::for_operation(
                operation,
                fence,
                "writing-start-status",
                cause,
            )))
        };
        let Some(owner) = self.owner.as_ref() else {
            return uncertain(None, "writing_start_owner_unmanaged");
        };
        let guard = match owner.inspection_guard() {
            Ok(guard) => guard,
            Err(_) => return uncertain(None, "writing_start_owner_unavailable"),
        };
        let snapshot = guard.snapshot();
        let cached = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = match self.raw.lock() {
            Ok(db) => db,
            Err(_) => return uncertain(Some(cached), "sql_mutex_poisoned"),
        };
        if !db.is_autocommit() {
            return uncertain(Some(cached), "sql_transaction_open");
        }
        let fence = match receipts::status_fence(&db, operation, cached, snapshot.accepted_position)
        {
            Ok(fence) => fence,
            Err(cause) => {
                return match cause.diagnostic() {
                    Some(detail) => WritingStartInspection::Unconfirmed(Box::new(detail.clone())),
                    None => uncertain(Some(cached), "writing_start_fence_unconfirmed"),
                };
            }
        };
        match writing_start_inspection::inspect(&db, intent) {
            Ok(fact)
                if snapshot.accepted_position > fence.position
                    && !matches!(fact, Fact::Started { .. }) =>
            {
                uncertain(Some(fence), "writing_start_pending_history_unconfirmed")
            }
            Ok(Fact::Missing) if snapshot.blocked => {
                uncertain(Some(fence), "owner_blocked_receipt_unconfirmed")
            }
            Ok(Fact::Missing) if snapshot.closed => {
                uncertain(Some(fence), "owner_closed_receipt_unconfirmed")
            }
            Ok(fact) => WritingStartInspection::Confirmed(fact),
            Err(cause) => {
                let mut detail = SqlImmediateDiagnostic::for_operation(
                    operation,
                    Some(fence),
                    "writing-start-status",
                    cause.safe_cause,
                );
                if let Some(error) = cause.sql {
                    detail = detail.sqlite(&error);
                }
                WritingStartInspection::Unconfirmed(Box::new(detail))
            }
        }
    }
}
