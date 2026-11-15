use super::*;
use crate::data_interface::{DatabaseGuard, guarded_transaction};
use rusqlite::{DropBehavior, Transaction, TransactionBehavior};

/// The two original Deferred bodies share settlement mechanics, not policy.
pub(super) fn run<T>(
    guard: &mut DatabaseGuard<'_>,
    body: impl FnOnce(&Transaction<'_>, &mut AttachmentProgress) -> Result<T>,
) -> Result<T> {
    guarded_transaction::run(guard, |db| {
        let mut progress = AttachmentProgress::new("begin");
        let result = (|| {
            let was_autocommit = db.is_autocommit();
            let mut tx =
                Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|e| {
                    if was_autocommit && !db.is_autocommit() {
                        progress.sql = AttachmentSql::Unknown;
                    }
                    progress.sql_error(&e)
                })?;
            tx.set_drop_behavior(DropBehavior::Ignore);
            progress.sql = AttachmentSql::StatementFailed;
            let value = match body(&tx, &mut progress) {
                Ok(value) => value,
                Err(mut error) => {
                    settle(db, tx.rollback(), &mut error);
                    return Err(error);
                }
            };
            progress.phase = "commit";
            if let Err(cause) = tx.commit() {
                let mut error = progress.sql_error(&cause);
                if db.is_autocommit() {
                    error.progress.sql = AttachmentSql::Unknown;
                } else {
                    settle(db, db.execute_batch("ROLLBACK"), &mut error);
                }
                return Err(error);
            }
            if !db.is_autocommit() {
                progress.sql = AttachmentSql::Unknown;
                return Err(progress.fail("storage_error", "附件事务未结算。"));
            }
            Ok(value)
        })();
        let unknown = result
            .as_ref()
            .is_err_and(|e| e.progress.sql == AttachmentSql::Unknown);
        (result, unknown)
    })
}

fn settle(db: &Connection, result: rusqlite::Result<()>, error: &mut AttachmentFailure) {
    match result {
        Ok(()) if db.is_autocommit() => error.progress.sql = AttachmentSql::RolledBack,
        result => {
            error.progress.sql = AttachmentSql::Unknown;
            error.rollback = Some(Box::new(match result {
                Err(cause) => {
                    crate::runtime_plugins::native::SafeDatabaseError::sql("rollback", &cause)
                }
                Ok(()) => crate::runtime_plugins::native::SafeDatabaseError::invariant(
                    "attachment_transaction_unsettled",
                    "rollback",
                ),
            }));
        }
    }
}
