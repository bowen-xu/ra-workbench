//! Canonical request preparation never writes cumulative response prefixes.
use super::*;
use crate::agent::model::responses_metadata::{
    CapturedResponsesInput, PreparedResponsesInput, ResponsesRequestMetadata,
};
use research_agent_core::model::ContextPersistenceDiagnostic;
use rusqlite::{DropBehavior, Transaction};

fn persistence_error(
    conversation_id: Uuid,
    phase: &'static str,
    safe_cause: &'static str,
    error: Option<&rusqlite::Error>,
) -> ModelError {
    let sqlite_extended_code = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => Some(code.extended_code),
        _ => None,
    };
    ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id,
        phase,
        safe_cause,
        sqlite_extended_code,
    })
}

pub(super) fn prepare(
    owner: &SqliteResponsesState,
    account: Uuid,
    model: ModelIdentity,
    input: Vec<Value>,
    captured: CapturedResponsesInput,
) -> ModelFuture<'_, PreparedResponsesInput> {
    let connection = owner.connection.clone();
    let scope = owner.scope.clone();
    let max_bytes = owner.max_bytes;
    let policy = owner.native_read_policy;
    Box::pin(async move {
        if account.is_nil() || model != scope.model {
            return Err(ModelError::InvalidRequest);
        }
        tokio::task::spawn_blocking(move || {
            let mut db = connection.lock().map_err(|_| ModelError::Persistence)?;
            crate::data_interface::guarded_transaction::run(&mut db, |db| {
                let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Immediate) {
                    Ok(tx) => tx,
                    Err(error) => {
                        return (
                            Err(persistence_error(
                                scope.conversation_id,
                                "begin",
                                "sql_begin_failed",
                                Some(&error),
                            )),
                            !db.is_autocommit(),
                        );
                    }
                };
                tx.set_drop_behavior(DropBehavior::Ignore);
                let operation = (|| -> Result<PreparedResponsesInput, ModelError> {
                    admission::validate_scope(&tx, &scope)?;
                    if captured.sampling {
                        admission::require_running_request(
                            &tx,
                            &scope,
                            captured.request_id,
                            &model,
                        )?;
                    }
                    if let Some(history) = &captured.history
                        && super::super::context_windows::read_active(&tx, scope.conversation_id)
                            .map_err(|_| ModelError::InvalidStream)?
                            .is_some_and(|window| window.source_watermark > history.after_seq)
                    {
                        return Err(ModelError::InvalidRequest);
                    }
                    let active: Option<String> = tx
                .query_row(
                    "select window_id from agent_active_context_windows where conversation_id=?1",
                    [scope.conversation_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| ModelError::Unavailable)?;
                    let legacy = legacy::prepare(
                        &tx,
                        &scope,
                        max_bytes,
                        account,
                        &input,
                        &captured,
                        active.as_deref(),
                        policy,
                    )?;
                    let prepared = match legacy {
                        Some(input) => input,
                        None => {
                            let state = ResponsesContinuation::new(
                                scope.turn_id,
                                model.clone(),
                                max_bytes,
                            )?;
                            compaction::bind_installed(&state, &tx, &scope, account, policy)?;
                            state.prepare_scoped_sync(
                                account,
                                model,
                                input,
                                Some(captured.authority_items),
                                captured.text_compaction,
                            )?
                        }
                    };
                    let identity = super::super::context_windows::identity::read_or_create_owned(
                        &tx,
                        scope.conversation_id,
                        Uuid::now_v7(),
                    )
                    .map_err(|error| match error {
                        super::super::context_windows::Error::Storage => ModelError::Persistence,
                        super::super::context_windows::Error::Canceled => ModelError::Canceled,
                        super::super::context_windows::Error::Inactive
                        | super::super::context_windows::Error::Conflict => {
                            ModelError::InvalidRequest
                        }
                        super::super::context_windows::Error::Invalid => ModelError::InvalidStream,
                    })?;
                    let metadata = ResponsesRequestMetadata::new(
                        scope.conversation_id,
                        scope.turn_id,
                        identity.number,
                    )?;
                    Ok(PreparedResponsesInput {
                        input: prepared,
                        metadata: Some(metadata),
                    })
                })();
                match operation {
                    Err(error) => {
                        let rollback = tx.rollback();
                        let unknown = rollback.is_err() || !db.is_autocommit();
                        let error = if unknown {
                            persistence_error(
                                scope.conversation_id,
                                "rollback",
                                if rollback.is_err() {
                                    "sql_rollback_failed"
                                } else {
                                    "sql_transaction_remains_open"
                                },
                                rollback.as_ref().err(),
                            )
                        } else {
                            error
                        };
                        (Err(error), unknown)
                    }
                    Ok(prepared) => {
                        if let Err(error) = tx.commit() {
                            if !db.is_autocommit() {
                                let _ = db.execute_batch("rollback");
                            }
                            return (
                                Err(persistence_error(
                                    scope.conversation_id,
                                    "commit",
                                    "sql_commit_outcome_unknown",
                                    Some(&error),
                                )),
                                true,
                            );
                        }
                        if !db.is_autocommit() {
                            return (
                                Err(persistence_error(
                                    scope.conversation_id,
                                    "commit",
                                    "sql_transaction_remains_open",
                                    None,
                                )),
                                true,
                            );
                        }
                        (Ok(prepared), false)
                    }
                }
            })
        })
        .await
        .map_err(|_| ModelError::Persistence)?
    })
}

#[path = "canonical/legacy.rs"]
mod legacy;
