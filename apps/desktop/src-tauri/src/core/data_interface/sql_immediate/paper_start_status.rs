//! Historical paper admissions are observed without starting or projecting work.
use super::*;
use crate::agent::{
    reading_start::StartRequest,
    repository::{SqlRepositoryError, submissions, submissions::start_observation},
};
use research_agent_core::protocol::{TaskKind, TurnOutcome};
use rusqlite::{OptionalExtension, params};

pub(crate) enum PaperStartInspection {
    Started {
        turn_id: Uuid,
        outcome: Option<TurnOutcome>,
    },
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
impl Database {
    pub(crate) fn inspect_paper_start(&self, request: &StartRequest) -> PaperStartInspection {
        let uncertain = |fence, cause| {
            PaperStartInspection::Unconfirmed(Box::new(SqlImmediateDiagnostic::for_operation(
                request.request_id,
                fence,
                "paper-start-status",
                cause,
            )))
        };
        let Some(owner) = self.owner.as_ref() else {
            return uncertain(None, "paper_start_owner_unmanaged");
        };
        let guard = match owner.inspection_guard() {
            Ok(guard) => guard,
            Err(_) => return uncertain(None, "paper_start_owner_unavailable"),
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
        let fence = match receipts::status_fence(
            &db,
            request.request_id,
            cached,
            snapshot.accepted_position,
        ) {
            Ok(fence) => fence,
            Err(error) => {
                return match error.diagnostic() {
                    Some(detail) => PaperStartInspection::Unconfirmed(Box::new(detail.clone())),
                    None => uncertain(Some(cached), "paper_start_fence_unconfirmed"),
                };
            }
        };
        let record = match submissions::read_sql(&db, request.conversation_id, request.request_id) {
            Ok(Some(record)) => record,
            Ok(None) => return uncertain(Some(fence), "paper_start_submission_unconfirmed"),
            Err(SqlRepositoryError::Sql(error)) => {
                return PaperStartInspection::Unconfirmed(Box::new(
                    SqlImmediateDiagnostic::for_operation(
                        request.request_id,
                        Some(fence),
                        "paper-start-status",
                        "paper_start_submission_unavailable",
                    )
                    .sqlite(&error),
                ));
            }
            Err(SqlRepositoryError::Domain(_)) => {
                return uncertain(Some(fence), "paper_start_submission_invalid");
            }
        };
        if record.context_snapshot_id != request.context_snapshot_id
            || record.input.model != request.model
        {
            return uncertain(Some(fence), "paper_start_intent_mismatch");
        }
        let scope: rusqlite::Result<Option<(String, String, String)>> = db.query_row(
            "select turn_id,paper_id,paper_context_snapshot_id from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2",
            params![request.conversation_id.to_string(), request.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional();
        match scope {
            Ok(Some((turn, paper, snapshot)))
                if turn == record.turn_id.to_string()
                    && paper == request.paper_id
                    && snapshot == request.paper_context_snapshot_id => {}
            Ok(_) => return uncertain(Some(fence), "paper_start_scope_unconfirmed"),
            Err(error) => {
                return PaperStartInspection::Unconfirmed(Box::new(
                    SqlImmediateDiagnostic::for_operation(
                        request.request_id,
                        Some(fence),
                        "paper-start-status",
                        "paper_start_scope_unavailable",
                    )
                    .sqlite(&error),
                ));
            }
        }
        match start_observation::observe(&db, &record, TaskKind::Regular) {
            Ok(Some(outcome)) => PaperStartInspection::Started {
                turn_id: record.turn_id,
                outcome,
            },
            Ok(None) => uncertain(Some(fence), "paper_start_worker_unconfirmed"),
            Err(
                start_observation::Error::Unavailable(error)
                | start_observation::Error::InvalidRow(error),
            ) => PaperStartInspection::Unconfirmed(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    request.request_id,
                    Some(fence),
                    "paper-start-status",
                    "paper_start_history_unavailable",
                )
                .sqlite(&error),
            )),
            Err(_) => uncertain(Some(fence), "paper_start_history_invalid"),
        }
    }
}
