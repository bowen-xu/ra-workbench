//! Persist the initial UUID separately; derive ordinal/previous from the
//! verified installed chain, never creation times or all historical rows.
use super::*;
use research_agent_core::model::ContextPersistenceDiagnostic;
use std::collections::HashSet;

#[cfg(test)]
#[path = "identity/managed_tests.rs"]
mod managed_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowIdentity {
    pub number: u64,
    pub first: Uuid,
    pub previous: Option<Uuid>,
    pub current: Uuid,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ManagedIdentityError {
    Domain(Error),
    Persistence(ContextPersistenceDiagnostic),
}

fn diagnostic(
    conversation_id: Uuid,
    phase: &'static str,
    safe_cause: &'static str,
    error: Option<&rusqlite::Error>,
) -> ManagedIdentityError {
    let sqlite_extended_code = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => Some(code.extended_code),
        _ => None,
    };
    ManagedIdentityError::Persistence(ContextPersistenceDiagnostic {
        conversation_id,
        phase,
        safe_cause,
        sqlite_extended_code,
    })
}

/// The host supplies UUIDv7 generation. Initial creation and chain validation
/// share one transaction, so a corrupt chain cannot publish a new identity.
#[cfg(test)]
pub(crate) fn read_or_create(
    db: &mut Connection,
    conversation: Uuid,
    initial: Uuid,
) -> Result<WindowIdentity, Error> {
    if conversation.is_nil() || initial.is_nil() || initial.get_version_num() != 7 {
        return Err(Error::Invalid);
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let identity = read_or_create_owned(&tx, conversation, initial)?;
    tx.commit()?;
    Ok(identity)
}

/// Keep the data owner fenced when the compatibility transaction's result is unknown.
pub(crate) fn read_or_create_managed(
    guard: &mut crate::data_interface::DatabaseGuard<'_>,
    conversation: Uuid,
    initial: Uuid,
) -> Result<WindowIdentity, ManagedIdentityError> {
    crate::data_interface::guarded_transaction::run(guard, |db| {
        let mut tx = match rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        {
            Ok(tx) => tx,
            Err(error) => {
                return (
                    Err(diagnostic(
                        conversation,
                        "begin",
                        "sql_begin_failed",
                        Some(&error),
                    )),
                    !db.is_autocommit(),
                );
            }
        };
        tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
        match read_or_create_owned(&tx, conversation, initial) {
            Err(error) => {
                let rollback = tx.rollback();
                let unknown = rollback.is_err() || !db.is_autocommit();
                (
                    Err(if unknown {
                        diagnostic(
                            conversation,
                            "rollback",
                            if rollback.is_err() {
                                "sql_rollback_failed"
                            } else {
                                "sql_transaction_remains_open"
                            },
                            rollback.as_ref().err(),
                        )
                    } else {
                        ManagedIdentityError::Domain(error)
                    }),
                    unknown,
                )
            }
            Ok(identity) => {
                if let Err(error) = tx.commit() {
                    if !db.is_autocommit() {
                        let _ = db.execute_batch("rollback");
                    }
                    return (
                        Err(diagnostic(
                            conversation,
                            "commit",
                            "sql_commit_outcome_unknown",
                            Some(&error),
                        )),
                        true,
                    );
                }
                if !db.is_autocommit() {
                    return (
                        Err(diagnostic(
                            conversation,
                            "commit",
                            "sql_transaction_remains_open",
                            None,
                        )),
                        true,
                    );
                }
                (Ok(identity), false)
            }
        }
    })
}

/// Caller owns the transaction, including any prepared model input.
pub(crate) fn read_or_create_owned(
    tx: &Connection,
    conversation: Uuid,
    initial: Uuid,
) -> Result<WindowIdentity, Error> {
    if conversation.is_nil() || initial.is_nil() || initial.get_version_num() != 7 {
        return Err(Error::Invalid);
    }
    let first: Option<String> = tx
        .query_row(
            "select window_id from agent_initial_context_windows where conversation_id=?1",
            [conversation.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let first = match first {
        Some(value) => Uuid::parse_str(&value).map_err(|_| Error::Invalid)?,
        None => {
            let live: bool = tx.query_row(
                "select exists(select 1 from agent_conversations where id=?1 and archived=0)",
                [conversation.to_string()],
                |row| row.get(0),
            )?;
            if !live {
                return Err(Error::Inactive);
            }
            tx.execute(
                "insert into agent_initial_context_windows(conversation_id,window_id) values(?1,?2)",
                params![conversation.to_string(), initial.to_string()],
            )?;
            initial
        }
    };
    if first.is_nil() || first.get_version_num() != 7 {
        return Err(Error::Invalid);
    }
    let active = read_active(tx, conversation)?;
    let has_pointer: bool = tx.query_row(
        "select exists(select 1 from agent_active_context_windows where conversation_id=?1)",
        [conversation.to_string()],
        |row| row.get(0),
    )?;
    if has_pointer != active.is_some() {
        return Err(Error::Invalid);
    }
    let mut identity = WindowIdentity {
        number: 0,
        first,
        previous: None,
        current: first,
    };
    let mut current = active.as_ref().map(|value| value.window_id);
    let mut seen = HashSet::new();
    let mut bytes = 0usize;
    let mut newer = u64::MAX;
    while let Some(id) = current {
        if id == first || !seen.insert(id) {
            return Err(Error::Invalid);
        }
        let checkpoint = fork::read_previous(tx, conversation, id).map_err(|error| {
            if error == research_agent_core::conversation::CatalogError::Unavailable {
                Error::Storage
            } else {
                Error::Invalid
            }
        })?;
        if checkpoint.source_watermark > newer {
            return Err(Error::Invalid);
        }
        newer = checkpoint.source_watermark;
        bytes = bytes
            .checked_add(
                serde_json::to_vec(&checkpoint)
                    .map_err(|_| Error::Invalid)?
                    .len(),
            )
            .ok_or(Error::Invalid)?;
        if bytes > MAX_CHECKPOINT_BYTES {
            return Err(Error::Invalid);
        }
        identity.number = identity.number.checked_add(1).ok_or(Error::Invalid)?;
        current = checkpoint.previous_window_id;
    }
    if let Some(active) = active {
        identity.current = active.window_id;
        identity.previous = Some(active.previous_window_id.unwrap_or(first));
    }
    Ok(identity)
}

#[cfg(test)]
#[path = "identity/tests.rs"]
mod tests;
