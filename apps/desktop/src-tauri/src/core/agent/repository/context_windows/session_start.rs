//! Durable adaptation of pinned Codex's queued SessionStartSource::Compact.
//! Enqueue is part of install; consume before executing, never replay side effects.
use super::*;

fn supported(db: &Connection) -> Result<bool, Error> {
    // Legacy migration fixtures intentionally exercise pre-hook schemas. The
    // application migrates before opening a session, so production has this table.
    Ok(db.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_compact_session_start')",
        [], |row| row.get(0),
    )?)
}

pub(super) fn clear(db: &Connection, conversation: Uuid) -> Result<(), Error> {
    if supported(db)? {
        db.execute(
            "delete from agent_compact_session_start where conversation_id=?1",
            [conversation.to_string()],
        )?;
    }
    Ok(())
}

pub(super) fn enqueue(db: &Connection, checkpoint: &ContextCheckpoint) -> Result<(), Error> {
    if supported(db)? {
        db.execute(
            "insert into agent_compact_session_start(conversation_id,window_id) values(?1,?2)",
            params![
                checkpoint.conversation_id.to_string(),
                checkpoint.window_id.to_string()
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn take(
    db: &mut Connection,
    conversation: Uuid,
) -> Result<Option<ContextCheckpoint>, Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = take_owned(&tx, conversation)?;
    tx.commit()?;
    Ok(result)
}

fn take_owned(tx: &Connection, conversation: Uuid) -> Result<Option<ContextCheckpoint>, Error> {
    let Some(active) = read_active(tx, conversation)? else {
        clear(tx, conversation)?;
        return Ok(None);
    };
    loop {
        let pending: Option<(i64, String)> = tx.query_row(
            "select ordinal,window_id from agent_compact_session_start where conversation_id=?1 order by ordinal limit 1",
            [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((ordinal, window_id)) = pending else {
            return Ok(None);
        };
        // Retain queued ancestors of the active window, but never revive a
        // checkpoint from a discarded branch after rollback or replacement.
        let candidate = Uuid::parse_str(&window_id).map_err(|_| Error::Invalid)?;
        let ancestor = is_ancestor(tx, conversation, active.window_id, candidate)?;
        tx.execute(
            "delete from agent_compact_session_start where ordinal=?1",
            [ordinal],
        )?;
        if !ancestor {
            continue;
        }
        let (encoded, schema): (String, u32) = tx.query_row(
            "select checkpoint_json,schema_version from agent_context_checkpoints where conversation_id=?1 and id=?2",
            params![conversation.to_string(), window_id], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if encoded.len() > MAX_CHECKPOINT_BYTES {
            return Err(Error::Invalid);
        }
        let window: ContextCheckpoint =
            serde_json::from_str(&encoded).map_err(|_| Error::Invalid)?;
        window
            .validate(MAX_CHECKPOINT_BYTES)
            .map_err(|_| Error::Invalid)?;
        if window.conversation_id != conversation
            || window.window_id.to_string() != window_id
            || window.schema_version != schema
        {
            return Err(Error::Invalid);
        }
        return Ok(Some(window));
    }
}

pub(crate) fn take_managed(
    guard: &mut crate::data_interface::DatabaseGuard<'_>,
    conversation: Uuid,
) -> Result<
    Option<ContextCheckpoint>,
    research_agent_core::context::checkpoint_store::CheckpointStoreError,
> {
    use research_agent_core::{
        context::checkpoint_store::CheckpointStoreError, model::ContextPersistenceDiagnostic,
    };
    let diagnostic = |phase, safe_cause, error: Option<&rusqlite::Error>| {
        let sqlite_extended_code = match error {
            Some(rusqlite::Error::SqliteFailure(code, _)) => Some(code.extended_code),
            _ => None,
        };
        CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id: conversation,
            phase,
            safe_cause,
            sqlite_extended_code,
        })
    };
    crate::data_interface::guarded_transaction::run(guard, |db| {
        let mut tx = match rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        {
            Ok(tx) => tx,
            Err(error) => {
                return (
                    Err(diagnostic("begin", "sql_begin_failed", Some(&error))),
                    !db.is_autocommit(),
                );
            }
        };
        tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
        match take_owned(&tx, conversation) {
            Err(original) => {
                let rollback = tx.rollback();
                let unknown = rollback.is_err() || !db.is_autocommit();
                let error = if unknown {
                    diagnostic(
                        "rollback",
                        if rollback.is_err() {
                            "sql_rollback_failed"
                        } else {
                            "sql_transaction_remains_open"
                        },
                        rollback.as_ref().err(),
                    )
                } else {
                    match original {
                        Error::Storage => CheckpointStoreError::Unavailable,
                        Error::Invalid => CheckpointStoreError::Invalid,
                        Error::Conflict => CheckpointStoreError::Conflict,
                        Error::Inactive => CheckpointStoreError::Inactive,
                        Error::Canceled => CheckpointStoreError::Canceled,
                    }
                };
                (Err(error), unknown)
            }
            Ok(result) => {
                if let Err(error) = tx.commit() {
                    if !db.is_autocommit() {
                        let _ = db.execute_batch("rollback");
                    }
                    return (
                        Err(diagnostic(
                            "commit",
                            "sql_commit_outcome_unknown",
                            Some(&error),
                        )),
                        true,
                    );
                }
                if !db.is_autocommit() {
                    return (
                        Err(diagnostic("commit", "sql_transaction_remains_open", None)),
                        true,
                    );
                }
                (Ok(result), false)
            }
        }
    })
}

#[cfg(test)]
#[path = "session_start_tests.rs"]
mod tests;
