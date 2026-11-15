//! Original transaction outcomes use the real SQL/log owner, never a codec stub.
use super::{fixture::Fixture, *};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn complete_fixture() -> Fixture {
    let fixture = Fixture::new();
    let mut snapshot = fixture.seed_nested();
    fixture.seed_reading_source();
    snapshot["anchorIds"] = json!(["anchor"]);
    snapshot["attachmentIds"] = json!(["attachment"]);
    fixture.seed_snapshot(&snapshot);
    fixture
}

fn clear(db: &rusqlite::Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[test]
fn nested_begin_is_not_started_and_preserves_the_callers_transaction() {
    let fixture = complete_fixture();
    let before = fixture.state();
    let captured = fixture.capture();
    let plan = fixture.prepared(&captured);
    let mut db = fixture.data.db.lock().unwrap();
    db.execute_batch(
        "BEGIN DEFERRED;
         update bibliographic_items set title='synthetic caller value' where id='source';",
    )
    .unwrap();
    let error = apply(&mut db, &fixture.resources, &captured, &plan).unwrap_err();
    assert_eq!(error.outcome, UndoOutcome::NotStarted);
    assert!(matches!(error.problem.phase, UndoPhase::Begin));
    assert!(error.rollback_error.is_none());
    assert!(!db.is_autocommit());
    assert_eq!(
        db.query_row(
            "select title from bibliographic_items where id='source'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "synthetic caller value"
    );
    // The undo adapter must not settle a transaction owned by this caller.
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.reopen().state(), before);
}

#[test]
fn denied_or_ignored_rollback_preserves_the_original_sql_failure_and_fences_owner() {
    for deny in [true, false] {
        let fixture = complete_fixture();
        let before = fixture.state();
        let captured = fixture.capture();
        let plan = fixture.prepared(&captured);
        let mut db = fixture.data.db.lock().unwrap();
        db.execute_batch(
            "create temp trigger synthetic_undo_rollback_fail before update of state on merge_operations
             when new.state='undone' begin
               select raise(abort,'private undo rollback failure');end;",
        )
        .unwrap();
        db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => {
                if deny {
                    Authorization::Deny
                } else {
                    Authorization::Ignore
                }
            }
            _ => Authorization::Allow,
        }));
        let error = apply(&mut db, &fixture.resources, &captured, &plan).unwrap_err();
        assert_eq!(error.outcome, UndoOutcome::Unknown);
        assert!(matches!(error.problem.phase, UndoPhase::Restore));
        let UndoCause::Data(cause) = &error.problem.cause else {
            panic!("rollback must retain the original mark-undone SQL failure");
        };
        assert_eq!(cause.resource_id, "literature.duplicate-undo.mark-undone");
        assert_eq!(cause.sqlite_extended_code, Some(1811));
        let rollback = error.rollback_error.as_ref().unwrap();
        assert_eq!(rollback.phase, "rollback");
        if deny {
            assert_eq!(rollback.sqlite_extended_code, Some(23));
        } else {
            assert_eq!(rollback.code, "duplicate_undo_transaction_unsettled");
            assert!(rollback.sqlite_extended_code.is_none());
        }
        assert!(!db.is_autocommit());
        // This late failure followed actual nested deletes in the open TX.
        assert_eq!(
            db.query_row("select count(*) from nal_statements", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private undo rollback failure")
        );
        clear(&db);
        drop(db);
        assert!(fixture.data.db.lock().is_err());
        // A real close of both owners and SQL/log reopen rolls back the TX.
        assert_eq!(fixture.reopen_fenced().state(), before);
    }
}

#[test]
fn ignored_commit_is_unknown_and_fences_owner_until_sql_log_cold_reopen() {
    let fixture = complete_fixture();
    let before = fixture.state();
    let captured = fixture.capture();
    let plan = fixture.prepared(&captured);
    let mut db = fixture.data.db.lock().unwrap();
    // Locked rusqlite 0.37 maps the fixed COMMIT to Unknown. These resources
    // contain no transaction controls; this targets only the caller's COMMIT.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let error = apply(&mut db, &fixture.resources, &captured, &plan).unwrap_err();
    assert_eq!(error.outcome, UndoOutcome::Unknown);
    assert!(matches!(error.problem.phase, UndoPhase::Commit));
    assert!(error.rollback_error.is_none());
    let UndoCause::Database(cause) = &error.problem.cause else {
        panic!("ignored COMMIT must report the original unsettled transaction");
    };
    assert_eq!(cause.code, "duplicate_undo_transaction_unsettled");
    assert_eq!(cause.phase, "commit");
    assert!(!db.is_autocommit());
    assert_eq!(
        db.query_row(
            "select state from merge_operations where id=?1",
            [OPERATION],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "undone"
    );
    clear(&db);
    drop(db);
    assert!(fixture.data.db.lock().is_err());
    assert_eq!(fixture.reopen_fenced().state(), before);
}

#[test]
fn commit_hook_rejection_is_unknown_even_when_sqlite_has_returned_to_autocommit() {
    let fixture = complete_fixture();
    let before = fixture.state();
    let captured = fixture.capture();
    let plan = fixture.prepared(&captured);
    let mut db = fixture.data.db.lock().unwrap();
    db.commit_hook(Some(|| true));
    let error = apply(&mut db, &fixture.resources, &captured, &plan).unwrap_err();
    assert_eq!(error.outcome, UndoOutcome::Unknown);
    assert!(matches!(error.problem.phase, UndoPhase::Commit));
    assert!(error.rollback_error.is_none());
    assert!(db.is_autocommit());
    let UndoCause::Database(cause) = &error.problem.cause else {
        panic!("commit hook must retain the original SQLite commit error");
    };
    assert_eq!(cause.phase, "commit");
    assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
    assert!(cause.sqlite_extended_code.is_some());
    db.commit_hook(None::<fn() -> bool>);
    drop(db);
    assert!(fixture.data.db.lock().is_err());
    assert_eq!(fixture.reopen_fenced().state(), before);
}

#[test]
fn denied_commit_with_confirmed_rollback_keeps_the_original_owner_usable() {
    let fixture = complete_fixture();
    let before = fixture.state();
    let captured = fixture.capture();
    let plan = fixture.prepared(&captured);
    let mut db = fixture.data.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = apply(&mut db, &fixture.resources, &captured, &plan).unwrap_err();
    assert_eq!(error.outcome, UndoOutcome::RolledBack);
    assert!(matches!(error.problem.phase, UndoPhase::Commit));
    assert!(error.rollback_error.is_none());
    assert!(db.is_autocommit());
    let UndoCause::Database(cause) = &error.problem.cause else {
        panic!("denied COMMIT must retain its original SQLite error");
    };
    assert_eq!(cause.phase, "commit");
    assert_eq!(cause.sqlite_extended_code, Some(23));
    clear(&db);
    drop(db);
    // state() takes the original owner's guard again: it is not fenced.
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.reopen().state(), before);
}
