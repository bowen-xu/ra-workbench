//! Original Deferred settlement through the existing Core unknown-result fence.
use super::*;
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction},
    runtime_plugins::native::SafeDatabaseError,
};
use rusqlite::{DropBehavior, TransactionBehavior};

pub(super) fn run<T>(
    db: &mut DatabaseGuard<'_>,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    guarded_transaction::run(db, |connection| {
        let was_autocommit = connection.is_autocommit();
        let mut tx = match Transaction::new_unchecked(connection, TransactionBehavior::Deferred) {
            Ok(tx) => tx,
            Err(error) => {
                let unknown = was_autocommit && !connection.is_autocommit();
                let error = if unknown {
                    unresolved(error.into(), None)
                } else {
                    error.into()
                };
                return (Err(error), unknown);
            }
        };
        tx.set_drop_behavior(DropBehavior::Ignore);
        let value = match operation(&tx) {
            Ok(value) => value,
            Err(error) => return failed(connection, error, tx.rollback()),
        };
        if let Err(error) = tx.commit() {
            if connection.is_autocommit() {
                return (Err(unresolved(error.into(), None)), true);
            }
            return failed(
                connection,
                error.into(),
                connection.execute_batch("ROLLBACK"),
            );
        }
        if !connection.is_autocommit() {
            return (
                Err(unresolved(
                    CaptureFailure::Technical("browser_capture_commit_unsettled".into()),
                    None,
                )),
                true,
            );
        }
        (Ok(value), false)
    })
}
fn failed<T>(
    connection: &Connection,
    error: CaptureFailure,
    rollback: rusqlite::Result<()>,
) -> (Result<T>, bool) {
    match rollback {
        Ok(()) if connection.is_autocommit() => (Err(error), false),
        result => {
            let rollback = result
                .err()
                .map(|error| SafeDatabaseError::sql("rollback", &error))
                .unwrap_or_else(|| {
                    SafeDatabaseError::invariant("browser_capture_rollback_unsettled", "rollback")
                });
            (Err(unresolved(error, Some(Box::new(rollback)))), true)
        }
    }
}
fn unresolved(cause: CaptureFailure, rollback: Option<Box<SafeDatabaseError>>) -> CaptureFailure {
    CaptureFailure::Unknown {
        cause: Box::new(cause),
        rollback,
    }
}
