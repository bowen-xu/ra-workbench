//! Inspect durable independent execution history without starting another worker.
use super::*;
use crate::agent::{
    context_session::start::Request,
    repository::{SqlRepositoryError, input_queue, submissions, submissions::start_observation},
};
use research_agent_core::protocol::{TaskKind, TurnOutcome};
use rusqlite::{OptionalExtension, params};

type QueueOriginRow = (String, Option<String>, Option<u64>, Option<String>);

pub(crate) enum ContextStartInspection {
    Admitted(Box<research_agent_core::submission::SubmissionRecord>),
    Started {
        turn_id: Uuid,
        outcome: Option<TurnOutcome>,
        owner_settled: bool,
    },
    LegacySettled {
        turn_id: Uuid,
        outcome: TurnOutcome,
    },
    LegacyObservedCandidate {
        turn_id: Uuid,
        outcome: Option<TurnOutcome>,
        fence: ReadFence,
    },
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}
impl Database {
    pub(crate) fn inspect_context_start(&self, request: &Request) -> ContextStartInspection {
        let uncertain = |fence, cause| {
            ContextStartInspection::Unconfirmed(Box::new(SqlImmediateDiagnostic::for_operation(
                request.request_id,
                fence,
                "context-start-status",
                cause,
            )))
        };
        let Some(owner) = self.owner.as_ref() else {
            return uncertain(None, "context_start_owner_unmanaged");
        };
        let Ok(guard) = owner.inspection_guard() else {
            return uncertain(None, "context_start_owner_unavailable");
        };
        let snapshot = guard.snapshot();
        let cached = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let Ok(db) = self.raw.lock() else {
            return uncertain(Some(cached), "sql_mutex_poisoned");
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
                    Some(detail) => ContextStartInspection::Unconfirmed(Box::new(detail.clone())),
                    None => uncertain(Some(cached), "context_start_fence_unconfirmed"),
                };
            }
        };
        let record = match submissions::read_sql(&db, request.conversation_id, request.request_id) {
            Ok(Some(record)) => record,
            Ok(None) => return uncertain(Some(fence), "context_start_submission_unconfirmed"),
            Err(SqlRepositoryError::Sql(error)) => {
                return ContextStartInspection::Unconfirmed(Box::new(
                    SqlImmediateDiagnostic::for_operation(
                        request.request_id,
                        Some(fence),
                        "context-start-status",
                        "context_start_submission_unavailable",
                    )
                    .sqlite(&error),
                ));
            }
            Err(SqlRepositoryError::Domain(_)) => {
                return uncertain(Some(fence), "context_start_submission_invalid");
            }
        };
        if record.context_snapshot_id != request.context_snapshot_id {
            return uncertain(Some(fence), "context_start_intent_mismatch");
        }
        // A queued turn has a separate immutable admission. Its identity must
        // match before the execution history can prove this original start.
        let origin: rusqlite::Result<Option<QueueOriginRow>> = db.query_row(
            "select o.input_id,p.input_id,a.expected_revision,a.result_json from agent_queue_submission_origins o left join agent_input_queue_previews p on p.input_id=o.input_id and p.conversation_id=o.conversation_id and p.request_id=o.request_id left join agent_input_queue_admissions a on a.input_id=o.input_id and a.conversation_id=o.conversation_id and a.request_id=o.request_id where o.conversation_id=?1 and o.request_id=?2",
            params![request.conversation_id.to_string(), request.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional();
        let mut legacy = false;
        let admitted = match (origin, request.queue) {
            (Ok(None), None) => {
                let evidence: rusqlite::Result<bool> = db.query_row("select exists(select 1 from agent_input_queue_previews where conversation_id=?1 and request_id=?2) or exists(select 1 from agent_input_queue_admissions where conversation_id=?1 and request_id=?2)", params![request.conversation_id.to_string(), request.request_id.to_string()], |row|row.get(0));
                if !matches!(evidence, Ok(false)) {
                    return uncertain(Some(fence), "context_start_queue_unconfirmed");
                }
                false
            }
            (Ok(Some((input, Some(preview), Some(revision), Some(json)))), Some(queue))
                if input == queue.input_id.to_string()
                    && preview == input
                    && revision == queue.expected_revision =>
            {
                match super::context_queue_admission::check_context_queue_admission(
                    &db, fence, request,
                ) {
                    Ok(true) => {}
                    Ok(false) => legacy = true,
                    Err(error) => {
                        return match error.diagnostic() {
                            Some(detail) => {
                                // The public status describes the original start request;
                                // the SQL receipt's derived identity is internal to this check.
                                let mut detail = detail.clone();
                                detail.operation_id = request.request_id;
                                ContextStartInspection::Unconfirmed(Box::new(detail))
                            }
                            None => {
                                uncertain(Some(fence), "context_start_admission_intent_unconfirmed")
                            }
                        };
                    }
                }
                if input_queue::validate_admission_snapshot(
                    &json,
                    request.conversation_id,
                    queue.input_id,
                    revision,
                )
                .is_err()
                {
                    return uncertain(Some(fence), "context_start_queue_snapshot_invalid");
                }
                true
            }
            (Err(error), _) => {
                return ContextStartInspection::Unconfirmed(Box::new(
                    SqlImmediateDiagnostic::for_operation(
                        request.request_id,
                        Some(fence),
                        "context-start-status",
                        "context_start_queue_unavailable",
                    )
                    .sqlite(&error),
                ));
            }
            _ => return uncertain(Some(fence), "context_start_queue_unconfirmed"),
        };
        match start_observation::observe(&db, &record, TaskKind::Regular) {
            Ok(Some(outcome)) => {
                let owner_settled = if outcome.is_some()
                    && !snapshot.blocked
                    && !snapshot.closed
                    && snapshot.accepted_position == fence.position
                {
                    match super::task_settlement::settled(
                        &db,
                        fence,
                        &super::TaskSettlement {
                            conversation_id: request.conversation_id,
                            request_id: request.request_id,
                            turn_id: record.turn_id,
                        },
                    ) {
                        Ok(value) => value,
                        Err(error) => {
                            return match error.diagnostic() {
                                Some(detail) => {
                                    let mut detail = detail.clone();
                                    detail.operation_id = request.request_id;
                                    ContextStartInspection::Unconfirmed(Box::new(detail))
                                }
                                None => uncertain(Some(fence), "task_settlement_unconfirmed"),
                            };
                        }
                    }
                } else {
                    false
                };
                if legacy {
                    return match (outcome, owner_settled) {
                        (Some(outcome), true) => ContextStartInspection::LegacySettled {
                            turn_id: record.turn_id,
                            outcome,
                        },
                        _ if !snapshot.blocked
                            && !snapshot.closed
                            && snapshot.accepted_position == fence.position =>
                        {
                            ContextStartInspection::LegacyObservedCandidate {
                                turn_id: record.turn_id,
                                outcome,
                                fence,
                            }
                        }
                        _ => uncertain(Some(fence), "context_start_admission_intent_unconfirmed"),
                    };
                }
                ContextStartInspection::Started {
                    turn_id: record.turn_id,
                    outcome,
                    owner_settled,
                }
            }
            Ok(None)
                if admitted
                    && !legacy
                    && snapshot.accepted_position == fence.position
                    && !snapshot.blocked
                    && !snapshot.closed =>
            {
                ContextStartInspection::Admitted(Box::new(record))
            }
            Ok(None) => uncertain(Some(fence), "context_start_worker_unconfirmed"),
            Err(
                start_observation::Error::Unavailable(error)
                | start_observation::Error::InvalidRow(error),
            ) => ContextStartInspection::Unconfirmed(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    request.request_id,
                    Some(fence),
                    "context-start-status",
                    "context_start_history_unavailable",
                )
                .sqlite(&error),
            )),
            Err(_) => uncertain(Some(fence), "context_start_history_invalid"),
        }
    }
}
