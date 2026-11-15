//! Fence prompt-fragment persistence and exact replay under the data owner.
use super::*;
use crate::data_interface::DatabaseGuard;
use research_agent_core::{
    context::checkpoint_store::CheckpointStoreError, model::ContextPersistenceDiagnostic,
};

fn diagnostic(
    conversation: Uuid,
    phase: &'static str,
    cause: &'static str,
    error: Option<&rusqlite::Error>,
) -> CheckpointStoreError {
    let sqlite_extended_code = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => Some(code.extended_code),
        _ => None,
    };
    CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id: conversation,
        phase,
        safe_cause: cause,
        sqlite_extended_code,
    })
}
fn domain(error: Error) -> CheckpointStoreError {
    match error {
        Error::Storage => CheckpointStoreError::Unavailable,
        Error::Invalid => CheckpointStoreError::Invalid,
        Error::Conflict => CheckpointStoreError::Conflict,
        Error::Inactive => CheckpointStoreError::Inactive,
        Error::Canceled => CheckpointStoreError::Canceled,
    }
}
pub(crate) fn record(
    guard: &mut DatabaseGuard<'_>,
    batch: &FragmentBatch,
) -> Result<Recording, CheckpointStoreError> {
    let conversation = batch.conversation_id;
    crate::data_interface::guarded_transaction::run(guard, |db| {
        let tx = match rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate) {
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
        let result = record_in(&tx, batch);
        if !matches!(result, Ok(Recording::Recorded)) {
            let rollback = tx.rollback();
            let unknown = rollback.is_err() || !db.is_autocommit();
            if unknown {
                return (
                    Err(diagnostic(
                        conversation,
                        "rollback",
                        if rollback.is_err() {
                            "sql_rollback_failed"
                        } else {
                            "sql_transaction_remains_open"
                        },
                        rollback.as_ref().err(),
                    )),
                    true,
                );
            }
            return (result.map_err(domain), false);
        }
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
        (Ok(Recording::Recorded), false)
    })
}
