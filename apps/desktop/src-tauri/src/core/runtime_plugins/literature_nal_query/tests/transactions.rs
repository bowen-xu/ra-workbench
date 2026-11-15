use super::{fixture::*, *};
use crate::runtime_plugins::native::SafeDatabaseError;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn clear(db: &rusqlite::Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[test]
fn nested_begin_failure_does_not_rollback_the_callers_uncommitted_values() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.execute_batch("BEGIN DEFERRED;update nal_query_collections set revision=8 where id=1;")
        .unwrap();
    let error = save(&mut db, &fixture.resources, &plan(), 8).unwrap_err();
    assert_eq!(error.outcome, QueryOutcome::NotStarted);
    assert!(!db.is_autocommit());
    assert_eq!(fixture.resources.read_raw(&db).unwrap().revision, 8);
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    assert_eq!(fixture.raw().revision, 0);
}

#[test]
fn conflict_with_failed_rollback_is_unknown_and_never_contains_retry_phrase() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = save(&mut db, &fixture.resources, &plan(), -1).unwrap_err();
    assert!(matches!(error.cause, QueryCause::Conflict));
    assert_eq!(
        error.outcome,
        QueryOutcome::Unknown {
            observed_revision: None
        }
    );
    assert!(!error.to_string().contains("查询集合已变化"));
    assert!(error.rollback_error.is_some());
    clear(&db);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().raw().revision, 0);
}

#[test]
fn decode_then_ignored_rollback_keeps_observation_without_claiming_commit() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.execute_batch("create trigger query_corrupt after update on nal_query_collections begin update nal_query_collections set definitions='null' where id=1;end;").unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let error = save(&mut db, &fixture.resources, &plan(), 0).unwrap_err();
    assert!(matches!(error.cause, QueryCause::InvalidStructure));
    assert_eq!(
        error.outcome,
        QueryOutcome::Unknown {
            observed_revision: Some(1)
        }
    );
    assert!(!db.is_autocommit());
    assert!(!error.to_string().contains("查询集合已变化"));
    clear(&db);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().raw().definitions_json, "[]");
}

#[test]
fn ignored_commit_is_unknown_and_fences_owner_until_pair_reopen() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    // Locked rusqlite 0.37 maps the fixed COMMIT statement to Unknown.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let error = save(&mut db, &fixture.resources, &plan(), 0).unwrap_err();
    assert_eq!(
        error.outcome,
        QueryOutcome::Unknown {
            observed_revision: Some(1)
        }
    );
    assert!(!db.is_autocommit());
    assert!(!error.to_string().contains("查询集合已变化"));
    clear(&db);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().raw().revision, 0);
}

#[test]
fn commit_denied_then_confirmed_rollback_keeps_owner_usable() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = save(&mut db, &fixture.resources, &plan(), 0).unwrap_err();
    assert_eq!(error.outcome, QueryOutcome::RolledBack);
    assert!(db.is_autocommit());
    assert!(!error.to_string().contains("查询集合已变化"));
    clear(&db);
    drop(db);
    assert_eq!(fixture.raw().revision, 0);
}

#[test]
fn commit_hook_rejection_without_rollback_receipt_is_not_inferred_from_autocommit() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.commit_hook(Some(|| true));
    let error = save(&mut db, &fixture.resources, &plan(), 0).unwrap_err();
    assert_eq!(
        error.outcome,
        QueryOutcome::Unknown {
            observed_revision: Some(1)
        }
    );
    assert!(db.is_autocommit());
    db.commit_hook(None::<fn() -> bool>);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().raw().revision, 0);
}

#[test]
fn only_confirmed_conflict_exposes_the_exact_retry_text_and_revision_is_not_item_id() {
    for outcome in [
        QueryOutcome::NotStarted,
        QueryOutcome::Committed { revision: 9 },
        QueryOutcome::Unknown {
            observed_revision: Some(9),
        },
    ] {
        let error = QueryFailure {
            cause: QueryCause::Conflict,
            outcome,
            rollback_error: None,
        };
        assert!(!error.to_string().contains("查询集合已变化"));
    }
    let error = QueryFailure {
        cause: QueryCause::Conflict,
        outcome: QueryOutcome::RolledBack,
        rollback_error: None,
    };
    assert_eq!(error.to_string(), "查询集合已变化，请重试");
    assert_eq!(
        serde_json::to_value(QueryOutcome::Committed { revision: 9 }).unwrap(),
        serde_json::json!({"status":"committed","revision":9})
    );
    assert_eq!(
        serde_json::to_value(QueryOutcome::Unknown {
            observed_revision: Some(9)
        })
        .unwrap(),
        serde_json::json!({"status":"unknown","observedRevision":9})
    );
}

#[test]
fn shared_sql_codec_preserves_old_save_json_and_omits_sql_and_private_values() {
    let error = SafeDatabaseError::sql("read", &rusqlite::Error::InvalidQuery);
    let old_path: &crate::runtime_plugins::literature_catalog::save::SafeDatabaseError = &error;
    let expected = serde_json::json!({"code":"storage_error","phase":"read","safeCause":"sqlite_operation_failed"});
    assert_eq!(serde_json::to_value(old_path).unwrap(), expected);
    let fixture = Fixture::new();
    let db = fixture.db.lock().unwrap();
    let sql = "select ??? private-query-sql";
    let cause = match db.prepare(sql) {
        Err(error) => error,
        Ok(_) => panic!("bad SQL unexpectedly compiled"),
    };
    let diagnostic = SafeDatabaseError::sql("prepare", &cause);
    let text = serde_json::to_string(&diagnostic).unwrap();
    assert!(!text.contains(sql));
    assert!(!text.contains("private-query-sql"));
    assert!(diagnostic.sqlite_code.is_some());
    assert!(diagnostic.sqlite_extended_code.is_some());
}
