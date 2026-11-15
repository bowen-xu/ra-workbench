use research_agent_core::conversation::ConversationId;
use rusqlite::{
    Connection, DropBehavior, OptionalExtension, Transaction, TransactionBehavior, params,
};
use uuid::Uuid;

use super::error::AiError;

pub(crate) fn request_cancel(connection: &Connection, run_id: &str) -> Result<bool, AiError> {
    let changed = connection.execute(
        "update ai_runs set cancel_requested = 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status in ('queued', 'resolving_scope', 'retrieving',
             'extracting_evidence', 'generating', 'validating')",
        [run_id],
    )?;
    if changed == 0
        && !connection.query_row(
            "select exists(select 1 from ai_runs where id = ?1)",
            [run_id],
            |row| row.get::<_, bool>(0),
        )?
    {
        return Err(AiError::InvalidInput("找不到 AI 运行".to_owned()));
    }
    Ok(changed == 1)
}

pub(crate) fn begin_stage(
    connection: &mut Connection,
    run_id: &str,
    stage_kind: &str,
    input_hash: Option<&str>,
) -> Result<String, AiError> {
    let transaction = connection.transaction()?;
    let cancel_requested = transaction.query_row(
        "select cancel_requested from ai_runs where id = ?1",
        [run_id],
        |row| row.get::<_, bool>(0),
    )?;
    if cancel_requested {
        return Err(AiError::Canceled);
    }
    let changed = transaction.execute(
        "update ai_runs set status = ?2,
             started_at = coalesce(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status in ('queued', 'resolving_scope', 'retrieving',
             'extracting_evidence', 'generating', 'validating')",
        params![run_id, stage_kind],
    )?;
    if changed != 1 {
        return Err(AiError::InvalidInput("AI 运行已终止".to_owned()));
    }
    let attempt: i64 = transaction.query_row(
        "select coalesce(max(attempt), 0) + 1 from ai_run_stages
         where run_id = ?1 and stage_kind = ?2",
        params![run_id, stage_kind],
        |row| row.get(0),
    )?;
    let stage_id = Uuid::new_v4().to_string();
    transaction.execute(
        "insert into ai_run_stages
         (id, run_id, stage_kind, attempt, status, input_hash, started_at, created_at)
         values (?1, ?2, ?3, ?4, 'running', ?5,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![stage_id, run_id, stage_kind, attempt, input_hash],
    )?;
    transaction.commit()?;
    Ok(stage_id)
}

pub(crate) struct StageCompletion<'a> {
    pub(crate) output_hash: Option<&'a str>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) duration_ms: i64,
}

pub(crate) fn complete_stage(
    connection: &Connection,
    stage_id: &str,
    completion: StageCompletion<'_>,
) -> Result<(), AiError> {
    let changed = connection.execute(
        "update ai_run_stages set status = 'succeeded', output_hash = ?2,
             input_tokens = ?3, output_tokens = ?4, duration_ms = ?5,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'",
        params![
            stage_id,
            completion.output_hash,
            completion.input_tokens,
            completion.output_tokens,
            completion.duration_ms
        ],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(AiError::Canceled)
    }
}

pub(crate) fn fail_stage(
    connection: &Connection,
    stage_id: &str,
    error_code: &str,
    error_message: &str,
    duration_ms: i64,
) -> Result<(), AiError> {
    let changed = connection.execute(
        "update ai_run_stages set status = 'failed', error_code = ?2, error_message = ?3,
             duration_ms = ?4, finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'",
        params![stage_id, error_code, error_message, duration_ms],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(AiError::Canceled)
    }
}

#[cfg(test)]
pub(crate) fn finish_run(
    connection: &mut Connection,
    run_id: &str,
    status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    usage: Option<(i64, i64, i64, i64)>,
) -> Result<(), AiError> {
    if !matches!(status, "needs_review" | "partial" | "failed" | "canceled") {
        return Err(AiError::InvalidInput("无效的 AI 运行终态".to_owned()));
    }
    let transaction = connection.transaction()?;
    finish_run_in(
        &transaction,
        run_id,
        status,
        error_code,
        error_message,
        usage,
    )?;
    transaction.commit()?;
    Ok(())
}

fn finish_run_in(
    transaction: &Transaction<'_>,
    run_id: &str,
    status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    usage: Option<(i64, i64, i64, i64)>,
) -> Result<(), AiError> {
    ensure_shared_run_outcome(transaction, run_id, status)?;
    let (input, output, hit, miss) = usage.unwrap_or((0, 0, 0, 0));
    let changed = transaction.execute(
        "update ai_runs set status = ?2, error_code = ?3, error_message = ?4,
             input_tokens = case when ?5 then ?6 else input_tokens end,
             output_tokens = case when ?5 then ?7 else output_tokens end,
             cache_hit_tokens = case when ?5 then ?8 else cache_hit_tokens end,
             cache_miss_tokens = case when ?5 then ?9 else cache_miss_tokens end,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status in ('queued', 'resolving_scope', 'retrieving',
             'extracting_evidence', 'generating', 'validating')",
        params![
            run_id,
            status,
            error_code,
            error_message,
            usage.is_some(),
            input,
            output,
            hit,
            miss
        ],
    )?;
    if changed != 1 {
        return Err(AiError::RunTerminalConflict {
            run_id: run_id.to_owned(),
        });
    }
    transaction.execute(
        "update ai_run_stages set status = case when ?2 = 'canceled' then 'canceled' else 'failed' end,
             error_code = ?3, error_message = ?4,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where run_id = ?1 and status in ('queued', 'running')",
        params![run_id, status, error_code, error_message],
    )?;
    let has_legacy_bindings: bool = transaction.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_legacy_ai_run_bindings')",
        [],
        |row| row.get(0),
    )?;
    if has_legacy_bindings {
        transaction.execute(
            "update agent_legacy_ai_run_bindings set terminal_status = ?2 where ai_run_id = ?1",
            params![run_id, status],
        )?;
    }
    Ok(())
}

/// Compatibility boundary until the complete terminal operation has a durable receipt.
pub(crate) fn finish_run_guarded(
    guard: &mut crate::data_interface::DatabaseGuard<'_>,
    run_id: &str,
    status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    usage: Option<(i64, i64, i64, i64)>,
) -> Result<(), AiError> {
    if !matches!(status, "needs_review" | "partial" | "failed" | "canceled") {
        return Err(AiError::InvalidInput("无效的 AI 运行终态".to_owned()));
    }
    if !guard.is_autocommit() {
        guard.require_recovery();
        return Err(terminal_unknown(
            run_id,
            "begin",
            "sql_transaction_already_open",
            None,
            None,
        ));
    }
    crate::data_interface::guarded_transaction::run(guard, |db| {
        finish_run_guarded_transaction(db, run_id, status, error_code, error_message, usage)
    })
}

fn finish_run_guarded_transaction(
    connection: &Connection,
    run_id: &str,
    status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    usage: Option<(i64, i64, i64, i64)>,
) -> (Result<(), AiError>, bool) {
    let mut tx = match Transaction::new_unchecked(connection, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(error) if connection.is_autocommit() => return (Err(error.into()), false),
        Err(error) => {
            return (
                Err(terminal_unknown(
                    run_id,
                    "begin",
                    "sql_begin_failed",
                    Some(&error),
                    None,
                )),
                true,
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    match finish_run_in(&tx, run_id, status, error_code, error_message, usage) {
        Err(cause) => {
            let rollback = tx.rollback();
            if rollback.is_err() || !connection.is_autocommit() {
                return (
                    Err(terminal_unknown(
                        run_id,
                        "rollback",
                        if rollback.is_err() {
                            "sql_rollback_failed"
                        } else {
                            "sql_transaction_remains_open"
                        },
                        rollback.as_ref().err(),
                        Some(cause.code()),
                    )),
                    true,
                );
            }
            (Err(cause), false)
        }
        Ok(()) => {
            if let Err(error) = tx.commit() {
                let cleanup_failed = !connection.is_autocommit()
                    && (connection.execute_batch("rollback").is_err()
                        || !connection.is_autocommit());
                return (
                    Err(terminal_unknown(
                        run_id,
                        "commit",
                        if cleanup_failed {
                            "sql_commit_unknown_cleanup_failed"
                        } else {
                            "sql_commit_outcome_unknown"
                        },
                        Some(&error),
                        None,
                    )),
                    true,
                );
            }
            if !connection.is_autocommit() {
                return (
                    Err(terminal_unknown(
                        run_id,
                        "commit",
                        "sql_commit_left_transaction_open",
                        None,
                        None,
                    )),
                    true,
                );
            }
            (Ok(()), false)
        }
    }
}

fn terminal_unknown(
    run_id: &str,
    phase: &'static str,
    safe_cause: &'static str,
    error: Option<&rusqlite::Error>,
    prior_code: Option<&'static str>,
) -> AiError {
    let (sqlite_code, sqlite_extended_code) = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => {
            (Some(format!("{:?}", code.code)), Some(code.extended_code))
        }
        _ => (None, None),
    };
    AiError::RunTerminalUnknown {
        run_id: run_id.to_owned(),
        phase,
        safe_cause,
        prior_code,
        sqlite_code,
        sqlite_extended_code,
    }
}

/// A migrated run is only allowed to publish its legacy terminal projection
/// after the shared Agent journal has published the corresponding terminal
/// outcome. Unmapped historical runs keep their old behavior.
fn ensure_shared_run_outcome(
    connection: &Connection,
    run_id: &str,
    legacy_status: &str,
) -> Result<(), AiError> {
    let has_bindings: bool = connection.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_legacy_ai_run_bindings')",
        [],
        |row| row.get(0),
    )?;
    if !has_bindings {
        return Ok(());
    }
    let Some((conversation_id, request_id, candidate_id)): Option<(String, String, Option<String>)> =
        connection
            .query_row(
                "select conversation_id, request_id, candidate_id from agent_legacy_ai_run_bindings where ai_run_id = ?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
    else {
        return Ok(());
    };
    let Some(turn_id): Option<String> = connection
        .query_row(
            "select turn_id from agent_submissions where conversation_id = ?1 and request_id = ?2",
            params![conversation_id, request_id],
            |row| row.get(0),
        )
        .optional()?
    else {
        return Err(AiError::TemporaryUnavailable);
    };
    let conversation_id = ConversationId::from_uuid(
        Uuid::parse_str(&conversation_id).map_err(|_| AiError::TemporaryUnavailable)?,
    );
    let turn_id = Uuid::parse_str(&turn_id).map_err(|_| AiError::TemporaryUnavailable)?;
    let outcome = crate::agent::repository::projection::read_turn_outcome(
        connection,
        conversation_id,
        turn_id,
    )?;
    let validation_failed = connection.query_row(
        "select exists(
            select 1 from ai_run_stages
            where run_id = ?1 and stage_kind = 'validating' and status = 'failed'
        )",
        [run_id],
        |row| row.get::<_, bool>(0),
    )?;
    match outcome {
        Some(outcome)
            if crate::agent::repository::legacy_ai_runs::status_for_outcome(
                outcome,
                candidate_id.is_some(),
                validation_failed,
            ) == Some(legacy_status) =>
        {
            Ok(())
        }
        Some(_) | None => Err(AiError::TemporaryUnavailable),
    }
}

#[cfg(test)]
#[path = "run_state_repository/shared_outcome_tests.rs"]
mod shared_outcome_tests;
