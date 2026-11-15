use super::*;
use crate::data_interface::guarded_transaction;
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

pub(crate) fn create_header(
    db: &mut DatabaseGuard<'_>,
    resources: &ImportJobsResources,
    job_id: &str,
    source_kind: &str,
    inputs: &[(String, String)],
    progress: &mut ImportJobsProgress,
) -> NativeResult<Vec<String>> {
    guarded_transaction::run(db, |connection| {
        let result = run(connection, resources, job_id, source_kind, inputs, progress);
        let unknown = matches!(progress.outcome, SqlOutcome::Unknown);
        (result, unknown)
    })
}

fn run(
    db: &Connection,
    resources: &ImportJobsResources,
    job_id: &str,
    source_kind: &str,
    inputs: &[(String, String)],
    progress: &mut ImportJobsProgress,
) -> NativeResult<Vec<String>> {
    let was_autocommit = db.is_autocommit();
    let mut tx =
        Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|cause| {
            if was_autocommit && !db.is_autocommit() {
                progress.outcome = SqlOutcome::Unknown;
            }
            progress.database("begin", &cause)
        })?;
    tx.set_drop_behavior(DropBehavior::Ignore);
    let result = (|| {
        let count = i64::try_from(inputs.len()).map_err(|_| progress.mismatch())?;
        resources
            .write_in(
                &tx,
                "create-header",
                vec![text(job_id), text(source_kind), DataValue::Integer(count)],
            )
            .map_err(|cause| progress.data(cause))?;
        let mut result_ids = Vec::with_capacity(inputs.len());
        for (input_label, raw_input) in inputs {
            let id = uuid::Uuid::new_v4().to_string();
            resources
                .write_in(
                    &tx,
                    "insert-pending",
                    vec![text(&id), text(job_id), text(input_label), text(raw_input)],
                )
                .map_err(|cause| progress.data(cause))?;
            result_ids.push(id);
        }
        Ok(result_ids)
    })();
    let result_ids = match result {
        Ok(ids) => ids,
        Err(error) => return Err(rollback(db, tx.rollback(), progress, error)),
    };
    if let Err(cause) = tx.commit() {
        if db.is_autocommit() {
            progress.outcome = SqlOutcome::Unknown;
            return Err(progress.database("commit", &cause));
        }
        let error = progress.database("commit", &cause);
        return Err(rollback(db, db.execute_batch("ROLLBACK"), progress, error));
    }
    if !db.is_autocommit() {
        progress.outcome = SqlOutcome::Unknown;
        return Err(progress.mismatch());
    }
    progress.outcome = SqlOutcome::Returned;
    Ok(result_ids)
}

fn rollback(
    db: &Connection,
    result: rusqlite::Result<()>,
    progress: &mut ImportJobsProgress,
    mut error: Box<NativeHostError>,
) -> Box<NativeHostError> {
    progress.outcome = match result {
        Ok(()) if db.is_autocommit() => SqlOutcome::RolledBack,
        Err(cause) => {
            error.save_rollback_error = Some(Box::new(
                crate::runtime_plugins::native::SafeDatabaseError::sql("rollback", &cause),
            ));
            SqlOutcome::Unknown
        }
        Ok(()) => SqlOutcome::Unknown,
    };
    progress.bind(error)
}
