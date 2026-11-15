use super::*;
use crate::data_interface::guarded_transaction;
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

pub(super) fn create(
    db: &mut DatabaseGuard<'_>,
    storage: &PdfStorage,
    job: &str,
    entries: &[PreparedPdfImportEntryV1],
    p: &mut ImportJobsProgress,
) -> NativeResult<()> {
    guarded_transaction::run(db, |db| {
        let result = run(db, storage, job, entries, p);
        if matches!(p.outcome, SqlOutcome::Unknown) {
            p.header_committed = None;
        }
        (
            result.map_err(|error| p.bind(error)),
            matches!(p.outcome, SqlOutcome::Unknown),
        )
    })
}
fn run(
    db: &Connection,
    storage: &PdfStorage,
    job: &str,
    entries: &[PreparedPdfImportEntryV1],
    p: &mut ImportJobsProgress,
) -> NativeResult<()> {
    let was_autocommit = db.is_autocommit();
    let mut tx =
        Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|cause| {
            if was_autocommit && !db.is_autocommit() {
                p.outcome = SqlOutcome::Unknown;
            }
            p.database("begin", &cause)
        })?;
    tx.set_drop_behavior(DropBehavior::Ignore);
    let result = (|| {
        let count = i64::try_from(entries.len()).map_err(|_| p.mismatch())?;
        storage
            .pdf
            .write_in(
                &tx,
                "create-header",
                vec![text(job), DataValue::Integer(count)],
            )
            .map_err(|cause| p.data(cause))?;
        for entry in entries {
            let id = uuid::Uuid::new_v4().to_string();
            storage
                .jobs
                .write_in(
                    &tx,
                    "insert-pending",
                    vec![
                        text(&id),
                        text(job),
                        text(&entry.input_label),
                        text(&entry.raw_input),
                    ],
                )
                .map_err(|cause| p.data(cause))?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        return Err(rollback(db, tx.rollback(), p, error));
    }
    if let Err(cause) = tx.commit() {
        if db.is_autocommit() {
            p.outcome = SqlOutcome::Unknown;
            return Err(p.database("commit", &cause));
        }
        let error = p.database("commit", &cause);
        return Err(rollback(db, db.execute_batch("ROLLBACK"), p, error));
    }
    if !db.is_autocommit() {
        p.outcome = SqlOutcome::Unknown;
        return Err(p.mismatch());
    }
    p.outcome = SqlOutcome::Returned;
    Ok(())
}
fn rollback(
    db: &Connection,
    result: rusqlite::Result<()>,
    p: &mut ImportJobsProgress,
    mut error: Box<NativeHostError>,
) -> Box<NativeHostError> {
    p.outcome = match result {
        Ok(()) if db.is_autocommit() => SqlOutcome::RolledBack,
        Err(cause) => {
            error.save_rollback_error =
                Some(Box::new(save::SafeDatabaseError::sql("rollback", &cause)));
            SqlOutcome::Unknown
        }
        Ok(()) => SqlOutcome::Unknown,
    };
    p.bind(error)
}
