//! Installation keeps cancellation, SQL commit and the data owner in one boundary.
use super::*;
use crate::agent::cancellation::AgentCancellation;
use crate::data_interface::DatabaseGuard;
use research_agent_core::{
    context::checkpoint_store::CheckpointStoreError, model::ContextPersistenceDiagnostic,
};
#[cfg(test)]
#[path = "install_managed/tests.rs"]
mod tests;

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
pub(crate) fn install(
    guard: &mut DatabaseGuard<'_>,
    checkpoint: &ContextCheckpoint,
    revision: u64,
    cancellation: Option<&AgentCancellation>,
) -> Result<Installation, CheckpointStoreError> {
    let conversation = checkpoint.conversation_id;
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
        let result = install_in(&tx, checkpoint, revision);
        if !matches!(result, Ok(Installation::Installed)) {
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
        // If cancellation wins, the uncommitted transaction is dropped and
        // rolls back; inspect autocommit before reporting known cancellation.
        let committed = match cancellation {
            Some(signal) => signal.commit_if_active(|| tx.commit()),
            None => Ok(tx.commit()),
        };
        match committed {
            Err(()) if db.is_autocommit() => (Err(CheckpointStoreError::Canceled), false),
            Err(()) => (
                Err(diagnostic(
                    conversation,
                    "rollback",
                    "sql_transaction_remains_open",
                    None,
                )),
                true,
            ),
            Ok(Err(error)) => {
                if !db.is_autocommit() {
                    let _ = db.execute_batch("rollback");
                }
                (
                    Err(diagnostic(
                        conversation,
                        "commit",
                        "sql_commit_outcome_unknown",
                        Some(&error),
                    )),
                    true,
                )
            }
            Ok(Ok(())) if !db.is_autocommit() => (
                Err(diagnostic(
                    conversation,
                    "commit",
                    "sql_transaction_remains_open",
                    None,
                )),
                true,
            ),
            Ok(Ok(())) => (Ok(Installation::Installed), false),
        }
    })
}
