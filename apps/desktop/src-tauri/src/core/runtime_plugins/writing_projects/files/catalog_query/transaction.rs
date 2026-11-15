//! Settle the original Deferred read before waiting for Native projection.
use super::super::super::failure;
use super::{QuerySql, capture};
use crate::{
    data_interface::guarded_transaction,
    runtime_plugins::native::{NativeHostError, NativeResult, SafeDatabaseError},
    storage::AppStore,
};
use research_workbench_native_sdk::writing_catalog_query::{QueryPlan, QueryRead, QuerySnapshot};
use rusqlite::{Connection, DropBehavior};
fn sql_error(key: &str, phase: &'static str, cause: &rusqlite::Error) -> Box<NativeHostError> {
    let diagnostic = SafeDatabaseError::sql(phase, cause);
    let mut error = failure("storage_error", diagnostic.to_string(), key);
    error.query_database_error = Some(Box::new(diagnostic));
    error
}
pub(super) fn capture_owned(
    store: &AppStore,
    sql: &QuerySql,
    plan: &QueryPlan,
    key: &str,
) -> NativeResult<QuerySnapshot> {
    let mut guard = store
        .connection()
        .map_err(|cause| failure("storage_error", cause.to_string(), key))?;
    guarded_transaction::run(&mut guard, |db| capture_in(db, sql, plan, key))
}
fn rollback(
    db: &Connection,
    mut error: Box<NativeHostError>,
    result: rusqlite::Result<()>,
) -> (NativeResult<QuerySnapshot>, bool) {
    let cause = match result {
        Ok(()) if db.is_autocommit() => return (Err(error), false),
        Ok(()) => SafeDatabaseError::invariant(
            "writing_note_catalog_read_unsettled",
            "catalog_query_rollback",
        ),
        Err(cause) => SafeDatabaseError::sql("catalog_query_rollback", &cause),
    };
    error.query_rollback_error = Some(Box::new(cause));
    (Err(error), true)
}
fn capture_in(
    db: &Connection,
    sql: &QuerySql,
    plan: &QueryPlan,
    key: &str,
) -> (NativeResult<QuerySnapshot>, bool) {
    let mut tx = match db.unchecked_transaction() {
        Ok(tx) => tx,
        Err(cause) => {
            return (
                Err(sql_error(key, "catalog_query_begin", &cause)),
                !db.is_autocommit(),
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let snapshot = match capture(&tx, sql, plan) {
        Ok(snapshot) => snapshot,
        Err(cause) => {
            return rollback(
                db,
                sql_error(key, "catalog_query_read", &cause),
                tx.rollback(),
            );
        }
    };
    if !matches!(
        snapshot.read,
        QueryRead::Ready { .. } | QueryRead::Excluded {}
    ) {
        return match tx.rollback() {
            Ok(()) if db.is_autocommit() => (Ok(snapshot), false),
            result => rollback(
                db,
                failure(
                    "storage_error",
                    "writing_note_catalog_read_rollback_unsettled",
                    key,
                ),
                result,
            ),
        };
    }
    match tx.commit() {
        Ok(()) if db.is_autocommit() => (Ok(snapshot), false),
        Ok(()) => (
            Err(failure(
                "storage_error",
                "writing_note_catalog_read_commit_unsettled",
                key,
            )),
            true,
        ),
        Err(cause) => {
            let error = sql_error(key, "catalog_query_commit", &cause);
            if db.is_autocommit() {
                return (Err(error), true);
            }
            rollback(db, error, db.execute_batch("ROLLBACK"))
        }
    }
}
