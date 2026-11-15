use super::*;

fn clear(db: &Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

fn project_request() -> Request {
    Request::SetProjects {
        item_id: "survivor".into(),
        project_ids: vec!["p-a".into()],
    }
}

#[test]
fn failed_nested_begin_never_settles_or_fences_the_callers_existing_transaction() {
    for request in [
        project_request(),
        Request::TrashMany {
            item_ids: vec!["survivor".into()],
        },
    ] {
        let fixture = Fixture::new();
        let before = fixture.state();
        let ids: Vec<String> = if matches!(&request, Request::TrashMany { .. }) {
            vec!["survivor".into()]
        } else {
            vec![]
        };
        let mut db = fixture.data.db.lock().unwrap();
        db.execute_batch(
            "BEGIN DEFERRED;
             update bibliographic_items set title='synthetic caller value' where id='survivor';",
        )
        .unwrap();
        let error = apply(&mut db, &fixture.resources, &request, &ids).unwrap_err();
        assert_eq!(error.phase, "begin");
        assert_eq!(error.outcome, LifecycleOutcome::NotDispatched);
        assert!(error.rollback_error.is_none());
        assert!(!db.is_autocommit());
        assert_eq!(
            db.query_row(
                "select title from bibliographic_items where id='survivor'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "synthetic caller value"
        );
        db.execute_batch("ROLLBACK").unwrap();
        drop(db);
        assert_eq!(fixture.state(), before);
        assert_eq!(fixture.reopen().state(), before);
    }
}

#[test]
fn denied_commit_is_confirmed_rolled_back_and_original_owner_remains_usable() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let mut db = fixture.data.db.lock().unwrap();
    // Locked rusqlite 0.37 reports this fixed COMMIT as Unknown. No resource
    // contains transaction control, so the callback targets the caller COMMIT.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = apply(&mut db, &fixture.resources, &project_request(), &[]).unwrap_err();
    assert_eq!(error.phase, "commit");
    assert_eq!(error.outcome, LifecycleOutcome::RolledBack);
    assert!(error.rollback_error.is_none());
    let LifecycleProblem::Database(cause) = error.problem else {
        panic!("the original COMMIT rejection must remain available");
    };
    assert_eq!(cause.phase, "commit");
    assert_eq!(cause.sqlite_extended_code, Some(23));
    assert!(db.is_autocommit());
    clear(&db);
    drop(db);
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.reopen().state(), before);
}

#[test]
fn refused_or_ignored_rollback_keeps_both_causes_and_fences_until_actual_reopen() {
    for refused in [true, false] {
        let fixture = Fixture::new();
        let before = fixture.state();
        let mut db = fixture.data.db.lock().unwrap();
        db.execute_batch(
            "create temp trigger synthetic_lifecycle_rollback before insert on project_items
             when new.project_id='p-b' begin
             select raise(fail,'private lifecycle rollback trigger text');end;",
        )
        .unwrap();
        db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => {
                if refused {
                    Authorization::Deny
                } else {
                    Authorization::Ignore
                }
            }
            _ => Authorization::Allow,
        }));
        let request = Request::SetProjects {
            item_id: "survivor".into(),
            project_ids: vec!["p-a".into(), "p-b".into()],
        };
        let error = apply(&mut db, &fixture.resources, &request, &[]).unwrap_err();
        assert_eq!(error.phase, "transaction");
        assert_eq!(error.outcome, LifecycleOutcome::Unknown);
        let LifecycleProblem::Data(cause) = &error.problem else {
            panic!("rollback must not replace the original INSERT failure");
        };
        assert_eq!(cause.resource_id, "literature.lifecycle.project-insert");
        assert_eq!(cause.sqlite_extended_code, Some(1811));
        let rollback = error.rollback_error.as_ref().unwrap();
        assert_eq!(rollback.phase, "rollback");
        if refused {
            assert_eq!(rollback.sqlite_extended_code, Some(23));
        } else {
            assert_eq!(rollback.code, "literature_lifecycle_unsettled");
            assert!(rollback.sqlite_extended_code.is_none());
        }
        assert!(!db.is_autocommit());
        assert_eq!(
            fixture::rows(
                &db,
                "select project_id from project_items where item_id='survivor' order by project_id",
            ),
            ["p-a"]
        );
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private lifecycle rollback trigger text")
        );
        let native = error.native(false);
        assert_eq!(native.code, "storage_error");
        assert!(native.message.contains("禁止自动重试"));
        assert!(!native.message.contains("本次操作未提交"));
        clear(&db);
        drop(db);
        assert!(fixture.data.db.lock().is_err());
        assert_eq!(fixture.reopen_fenced().state(), before);
    }
}

#[test]
fn ignored_commit_keeps_unsettled_writes_unknown_and_never_reports_success() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let mut db = fixture.data.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let error = apply(&mut db, &fixture.resources, &project_request(), &[]).unwrap_err();
    assert_eq!(error.phase, "commit");
    assert_eq!(error.outcome, LifecycleOutcome::Unknown);
    assert!(error.rollback_error.is_none());
    assert!(matches!(
        error.problem,
        LifecycleProblem::Owner("connection_unsettled")
    ));
    assert!(!db.is_autocommit());
    assert_eq!(
        fixture::rows(
            &db,
            "select project_id from project_items where item_id='survivor' order by project_id",
        ),
        ["p-a"]
    );
    clear(&db);
    drop(db);
    assert!(fixture.data.db.lock().is_err());
    assert_eq!(fixture.reopen_fenced().state(), before);
}

#[test]
fn commit_hook_error_remains_unknown_even_when_sqlite_has_rolled_back_itself() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let mut db = fixture.data.db.lock().unwrap();
    db.commit_hook(Some(|| true));
    let error = apply(&mut db, &fixture.resources, &project_request(), &[]).unwrap_err();
    assert_eq!(error.phase, "commit");
    assert_eq!(error.outcome, LifecycleOutcome::Unknown);
    assert!(error.rollback_error.is_none());
    let LifecycleProblem::Database(cause) = error.problem else {
        panic!("the original commit-hook SQLite cause must be retained");
    };
    assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
    assert!(cause.sqlite_extended_code.is_some());
    assert!(db.is_autocommit());
    db.commit_hook(None::<fn() -> bool>);
    drop(db);
    assert!(fixture.data.db.lock().is_err());
    assert_eq!(fixture.reopen_fenced().state(), before);
}
