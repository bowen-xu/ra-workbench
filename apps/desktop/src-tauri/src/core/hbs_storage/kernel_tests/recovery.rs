use super::*;
use crate::data_interface::{Database, guarded_transaction};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn denied_rollback_returns_no_pause_and_blocks_original_database_log_owner() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let log = root.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let (id, before) = seeded(&mut sql);
    let database = Database::open(sql, &log).unwrap();
    let mut sources = Sources::new();
    sources.ready = false;
    {
        let mut guard = database.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
        let result = guarded_transaction::run_mut(&mut guard, |db| {
            let result = commit_with(&mut sources, db, "structured", &id, &request(&before));
            let unknown = matches!(&result,Err(BoundaryError::Failure(f)) if f.needs_recovery());
            (result, unknown)
        });
        let BoundaryError::Failure(f) = result.unwrap_err() else {
            panic!("uncertain rollback must not escape as Need")
        };
        assert_eq!(f.outcome, HbsOutcome::Unknown);
        assert!(f.rollback_error.is_some());
        assert!(f.needs_recovery());
        assert!(!guard.is_autocommit());
        assert_eq!(sources.policy.calls, 0);
    }
    assert!(database.lock().is_err());
    drop(database);
    let reopened = Database::open(Connection::open(&path).unwrap(), &log).unwrap();
    let guard = reopened.lock().unwrap();
    assert_eq!(read(&guard, "structured", &id).unwrap(), before);
    assert_eq!(
        guard
            .query_row("select count(*) from hbs_commit_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn policy_unwind_is_fenced_by_existing_guard_and_cold_reopen_keeps_original_authority() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let log = root.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let (id, before) = seeded(&mut sql);
    let database = Database::open(sql, &log).unwrap();
    let mut sources = Sources::new();
    sources.policy.panic = true;
    {
        let mut guard = database.lock().unwrap();
        let result = catch_unwind(AssertUnwindSafe(|| {
            guarded_transaction::run_mut(&mut guard, |db| {
                let result = commit_with(&mut sources, db, "structured", &id, &request(&before));
                (result, false)
            })
        }));
        assert!(result.is_err());
    }
    assert!(database.lock().is_err());
    drop(database);
    let reopened = Database::open(Connection::open(&path).unwrap(), &log).unwrap();
    let guard = reopened.lock().unwrap();
    assert_eq!(read(&guard, "structured", &id).unwrap(), before);
}
