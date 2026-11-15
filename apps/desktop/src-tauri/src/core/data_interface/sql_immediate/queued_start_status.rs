//! Read-only queue start inspection under the existing data owner and SQL lock.
use super::*;
use crate::agent::repository::input_queue::start_inspection::{self, Fact, QueueStartIntent};

pub(crate) enum QueueStartInspection {
    Confirmed(Fact),
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
impl Database {
    pub(crate) fn inspect_queue_start(&self, intent: &QueueStartIntent) -> QueueStartInspection {
        let operation = intent.request_id;
        let uncertain = |fence: Option<ReadFence>, cause| {
            QueueStartInspection::Unconfirmed(Box::new(SqlImmediateDiagnostic::for_operation(
                operation,
                fence,
                "queue-start-status",
                cause,
            )))
        };
        let Some(owner) = self.owner.as_ref() else {
            return uncertain(None, "queue_start_owner_unmanaged");
        };
        let guard = match owner.inspection_guard() {
            Ok(guard) => guard,
            Err(_) => return uncertain(None, "queue_start_owner_unavailable"),
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
                    Some(detail) => QueueStartInspection::Unconfirmed(Box::new(detail.clone())),
                    None => uncertain(Some(cached), "queue_start_fence_unconfirmed"),
                };
            }
        };
        match start_inspection::inspect(&db, intent, fence) {
            Ok(fact)
                if snapshot.accepted_position > fence.position
                    && !matches!(fact, Fact::Started { .. }) =>
            {
                uncertain(Some(fence), "queue_start_pending_history_unconfirmed")
            }
            Ok(Fact::Missing) if snapshot.blocked => {
                uncertain(Some(fence), "owner_blocked_receipt_unconfirmed")
            }
            Ok(Fact::Missing) if snapshot.closed => {
                uncertain(Some(fence), "owner_closed_receipt_unconfirmed")
            }
            Ok(fact) => QueueStartInspection::Confirmed(fact),
            Err(cause) => {
                let mut detail = SqlImmediateDiagnostic::for_operation(
                    operation,
                    Some(fence),
                    "queue-start-status",
                    cause.safe_cause,
                );
                if let Some(error) = cause.sql {
                    detail = detail.sqlite(&error);
                }
                QueueStartInspection::Unconfirmed(Box::new(detail))
            }
        }
    }
}
