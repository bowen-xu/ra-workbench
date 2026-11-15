use super::{fixture::*, *};
use crate::runtime_plugins::RuntimePluginCatalog;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::{
    fs,
    sync::{Arc, Mutex},
};

fn no_authorizer(db: &rusqlite::Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[test]
fn probe_uses_begin_rollback_and_failed_nested_begin_preserves_caller_transaction() {
    let fixture = Fixture::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&events);
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(move |context: AuthContext<'_>| {
        if let AuthAction::Transaction { operation } = context.action {
            received.lock().unwrap().push(format!("{operation:?}"));
        }
        Authorization::Allow
    }));
    probe_begin(&mut db).unwrap();
    assert!(db.is_autocommit());
    assert_eq!(*events.lock().unwrap(), ["Begin", "Rollback"]);
    no_authorizer(&db);
    db.execute_batch("BEGIN DEFERRED; insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('caller','document','uncommitted','before','before');").unwrap();
    let error = probe_begin(&mut db).unwrap_err();
    assert_eq!(error.outcome, SaveOutcome::NotStarted);
    assert!(!db.is_autocommit());
    assert_eq!(
        db.query_row(
            "select title from bibliographic_items where id='caller'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "uncommitted"
    );
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    assert_eq!(fixture.count("bibliographic_items"), 0);
}

#[test]
fn repeated_tags_roll_back_all_mutations_and_sql_log_reopens() {
    let fixture = Fixture::new();
    fixture.persist(&plan(Some("before"))).unwrap();
    let mut value = plan(Some("after"));
    value.tags = vec!["duplicate".into(), "duplicate".into()];
    let failure = fixture.persist(&value).unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::RolledBack);
    assert!(failure.rollback_error.is_none());
    let SaveCause::Data(cause) = failure.cause else {
        panic!("expected original tag SQL error");
    };
    assert_eq!(cause.resource_id, "literature.catalog-save.item-tag-insert");
    assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
    assert!(fixture.db.lock().unwrap().is_autocommit());
    let fixture = fixture.reopen();
    assert_eq!(fixture.count("bibliographic_items"), 1);
    assert_eq!(fixture.count("creators"), 1);
    assert_eq!(fixture.count("item_tags"), 2);
    assert_eq!(fixture.count("metadata_provenance"), 17);
    assert_eq!(fixture.count("literature_search"), 1);
}

#[test]
fn rollback_failure_keeps_original_error_blocks_admission_and_reopen_rolls_back() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let mut value = plan(Some("unknown"));
    value.tags = vec!["duplicate".into(), "duplicate".into()];
    let failure = persist(&mut db, &fixture.resources, &fixture.catalog, &value).unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::Unknown);
    let SaveCause::Data(cause) = &failure.cause else {
        panic!("rollback cannot replace the original SQL error");
    };
    assert_eq!(cause.resource_id, "literature.catalog-save.item-tag-insert");
    assert_eq!(failure.rollback_error.as_ref().unwrap().phase, "rollback");
    assert!(!db.is_autocommit());
    no_authorizer(&db);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().count("bibliographic_items"), 0);
}

#[test]
fn ignored_probe_rollback_is_unknown_and_fences_the_original_owner() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let failure = probe_begin(&mut db).unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::Unknown);
    assert!(!db.is_autocommit());
    no_authorizer(&db);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().count("bibliographic_items"), 0);
}

#[test]
fn ignored_successful_commit_never_becomes_a_committed_reply() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    // Locked rusqlite 0.37 maps COMMIT to Unknown; these fixed resources contain
    // no transaction controls, so this is the caller's explicit COMMIT only.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let failure = persist(
        &mut db,
        &fixture.resources,
        &fixture.catalog,
        &plan(Some("not-committed")),
    )
    .unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::Unknown);
    assert!(!db.is_autocommit());
    let SaveCause::Database(cause) = failure.cause else {
        panic!("expected unsettled commit");
    };
    assert_eq!(cause.code, "catalog_save_transaction_unsettled");
    no_authorizer(&db);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().count("bibliographic_items"), 0);
}

#[test]
fn failed_commit_without_rollback_receipt_stays_unknown_even_if_autocommit_is_true() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.commit_hook(Some(|| true));
    let failure = persist(
        &mut db,
        &fixture.resources,
        &fixture.catalog,
        &plan(Some("commit-hook")),
    )
    .unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::Unknown);
    assert!(db.is_autocommit());
    let SaveCause::Database(cause) = failure.cause else {
        panic!("expected commit error");
    };
    assert_eq!(cause.phase, "commit");
    assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
    db.commit_hook(None::<fn() -> bool>);
    drop(db);
    assert!(fixture.db.lock().is_err());
    assert_eq!(fixture.reopen().count("bibliographic_items"), 0);
}

#[test]
fn known_commit_rejection_can_explicitly_roll_back_and_keep_owner_usable() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    // COMMIT is Unknown in the locked hook API, with no other unknown transaction
    // statement on this fixed Deferred/COMMIT/ROLLBACK save path.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Unknown,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let failure = persist(
        &mut db,
        &fixture.resources,
        &fixture.catalog,
        &plan(Some("rejected")),
    )
    .unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::RolledBack);
    assert!(db.is_autocommit());
    no_authorizer(&db);
    drop(db);
    assert_eq!(fixture.count("bibliographic_items"), 0);
}

#[test]
fn safe_errors_keep_sqlite_codes_without_sql_trigger_text_or_bound_values() {
    let fixture = Fixture::new();
    fixture.db.lock().unwrap().execute_batch("create trigger synthetic_private_failure before insert on bibliographic_items begin select raise(abort,'private-trigger-token'); end;").unwrap();
    let mut value = plan(Some("safe-id"));
    value.item.title = "private-bound-title-token".into();
    let failure = fixture.persist(&value).unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::RolledBack);
    let display = failure.to_string();
    let serialized = serde_json::to_string(&failure).unwrap();
    for text in [display, serialized, format!("{failure:?}")] {
        assert!(!text.contains("private-trigger-token"));
        assert!(!text.contains("private-bound-title-token"));
        assert!(!text.contains("insert into bibliographic_items"));
    }
    let SaveCause::Data(cause) = failure.cause else {
        panic!("expected original trigger failure");
    };
    assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
    assert!(cause.sqlite_extended_code.is_some());
}

#[test]
fn save_and_return_resources_must_belong_to_the_same_package_and_exact_hashes() {
    let fixture = Fixture::new();
    let (_, other_reads) = package(fixture.directory.path(), "other");
    let mut db = fixture.db.lock().unwrap();
    let failure = persist(&mut db, &fixture.resources, &other_reads, &plan(None)).unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::NotStarted);
    assert!(db.is_autocommit());
    drop(db);
    assert_eq!(fixture.count("bibliographic_items"), 0);
    let root = fixture.directory.path().join("primary/literature");
    let file = root.join("data/catalog-save-item-upsert.v1.sql");
    let mut sql = fs::read(&file).unwrap();
    sql.push(b'\n');
    fs::write(file, sql).unwrap();
    let catalog = RuntimePluginCatalog::new(Some(fixture.directory.path().join("primary")));
    let selected = catalog.discover().unwrap().packages.remove(0);
    let error = match SaveResources::load(&catalog, &selected) {
        Ok(_) => panic!("changed SQL must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.code, "runtime_plugin_data_digest_mismatch");
}

#[test]
fn diagnostic_outcome_json_has_exact_status_and_original_committed_item_id() {
    assert_eq!(
        serde_json::to_value(SaveOutcome::NotStarted).unwrap(),
        serde_json::json!({"status":"notStarted"})
    );
    assert_eq!(
        serde_json::to_value(SaveOutcome::RolledBack).unwrap(),
        serde_json::json!({"status":"rolledBack"})
    );
    assert_eq!(
        serde_json::to_value(SaveOutcome::Unknown).unwrap(),
        serde_json::json!({"status":"unknown"})
    );
    assert_eq!(
        serde_json::to_value(SaveOutcome::Committed {
            item_id: " exact id ".into()
        })
        .unwrap(),
        serde_json::json!({"status":"committed","itemId":" exact id "})
    );
}
