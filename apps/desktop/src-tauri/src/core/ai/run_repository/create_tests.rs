use super::*;
use crate::ai::candidate_test_fixture::{
    TestDirectory, database, insert_selection_source, selection_draft,
};

use crate::ai::candidate_test_fixture::preview as preview_task;
fn no_portrait(_: &Transaction<'_>, _: &str, _: Uuid) -> Result<Uuid, AiError> {
    Err(AiError::CapabilityUnavailable(
        "unused portrait test source",
    ))
}
fn insert_run_in(
    tx: &Transaction<'_>,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
    shared: Option<(Uuid, Uuid)>,
    ids: &mut RunCreateIdentities,
) -> Result<AiRun, AiError> {
    ids.validate(draft, shared)?;
    let mut preview = preview_task(tx, credentials, draft)?;
    bind_confirmed_preview(draft, ids, &mut preview)?;
    records::insert_run_records(no_portrait, tx, draft, shared, ids, &preview, None)
}
fn insert_run_guarded(
    db: &mut DatabaseGuard<'_>,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
    shared: Option<(Uuid, Uuid)>,
) -> Result<AiRun, AiError> {
    let mut ids = RunCreateIdentities::new(draft, Uuid::new_v4(), shared)?;
    insert_prepared_run_guarded(
        db,
        draft,
        shared,
        &mut ids,
        |connection| preview_task(connection, credentials, draft),
        no_portrait,
    )
}

#[test]
fn selection_run_reuses_generated_identities_after_confirmed_rollback() {
    let mut db = database();
    let (anchor, _) = insert_selection_source(&db);
    let directory = TestDirectory::new();
    let mut draft = selection_draft(&anchor);
    draft.confirmed_scope_hash = Some(
        preview_task(&db, &directory.credentials(), &draft)
            .unwrap()
            .scope_hash,
    );
    let mut identities = RunCreateIdentities::new(&draft, Uuid::new_v4(), None).unwrap();
    let mut changed = draft.clone();
    changed.user_goal.push_str(" changed");
    let tx = db.transaction().unwrap();
    assert!(
        matches!(insert_run_in( &tx, &directory.credentials(), &changed, None, &mut identities),
        Err(AiError::InvalidInput(message)) if message == "AI 运行请求已变化，请重新准备")
    );
    assert!(
        matches!(insert_run_in( &tx, &directory.credentials(), &draft, Some((Uuid::new_v4(), Uuid::new_v4())), &mut identities),
        Err(AiError::InvalidInput(message)) if message == "AI 运行请求已变化，请重新准备")
    );
    let count: i64 = tx
        .query_row("select count(*) from ai_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    tx.rollback().unwrap();
    let tx = db.transaction().unwrap();
    let first =
        insert_run_in(&tx, &directory.credentials(), &draft, None, &mut identities).unwrap();
    tx.rollback().unwrap();
    let tx = db.transaction().unwrap();
    let second =
        insert_run_in(&tx, &directory.credentials(), &draft, None, &mut identities).unwrap();
    tx.commit().unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.shared_conversation_id, second.shared_conversation_id);
    assert_eq!(first.shared_request_id, second.shared_request_id);
    assert!(first.shared_conversation_id.is_some());
    assert_eq!(
        first
            .scope_items
            .iter()
            .map(|item| &item.id)
            .collect::<Vec<_>>(),
        second
            .scope_items
            .iter()
            .map(|item| &item.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn prepared_run_rejects_changed_model_after_confirmed_rollback() {
    let mut db = database();
    let (anchor, _) = insert_selection_source(&db);
    let directory = TestDirectory::new();
    let mut draft = selection_draft(&anchor);
    draft.confirmed_scope_hash = Some(
        preview_task(&db, &directory.credentials(), &draft)
            .unwrap()
            .scope_hash,
    );
    let mut identities = RunCreateIdentities::new(&draft, Uuid::new_v4(), None).unwrap();
    let tx = db.transaction().unwrap();
    let first =
        insert_run_in(&tx, &directory.credentials(), &draft, None, &mut identities).unwrap();
    tx.rollback().unwrap();

    db.execute(
        "update ai_provider_settings set model_id = 'fixture-v2' where provider_id = 'fixture-generation'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        insert_run_in( &tx, &directory.credentials(), &draft, None, &mut identities),
        Err(AiError::InvalidInput(message)) if message == "AI 模型配置已变化，请重新准备"
    ));
    let count: i64 = tx
        .query_row("select count(*) from ai_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    tx.rollback().unwrap();

    db.execute(
        "update ai_provider_settings set model_id = 'fixture-v1' where provider_id = 'fixture-generation'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    let retried =
        insert_run_in(&tx, &directory.credentials(), &draft, None, &mut identities).unwrap();
    tx.commit().unwrap();
    assert_eq!(first.id, retried.id);
    assert_eq!(first.model_id, retried.model_id);
    assert_eq!(first.shared_request_id, retried.shared_request_id);
}

#[test]
fn guarded_creation_blocks_owner_when_commit_fails() {
    use crate::data_interface::Database;

    let directory = tempfile::tempdir().unwrap();
    let sqlite_path = directory.path().join("data.sqlite");
    let log_path = directory.path().join("log");
    let mut sql = Connection::open(&sqlite_path).unwrap();
    sql.execute_batch("pragma foreign_keys=on").unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let (anchor, _) = insert_selection_source(&sql);
    let credentials = TestDirectory::new();
    let mut draft = selection_draft(&anchor);
    draft.confirmed_scope_hash = Some(
        preview_task(&sql, &credentials.credentials(), &draft)
            .unwrap()
            .scope_hash,
    );
    let db = Database::open(sql, &log_path).unwrap();
    let mut guard = db.lock().unwrap();
    guard.execute_batch(
        "create temp table fault_parent(id text primary key);
         create temp table fault_child(id text references fault_parent(id) deferrable initially deferred);
         create temp trigger fault_ai_run after insert on main.ai_runs
         begin insert into fault_child values ('missing'); end;",
    ).unwrap();
    let error = insert_run_guarded(&mut guard, &credentials.credentials(), &draft, None)
        .expect_err("deferred foreign key must fail at commit");
    let run_id = match &error {
        AiError::RunCreationUnknown {
            run_id,
            phase,
            sqlite_code,
            sqlite_extended_code,
            safe_cause,
            prior_code,
        } => {
            assert_eq!(*phase, "commit");
            assert_eq!(*safe_cause, "sql_commit_outcome_unknown");
            assert_eq!(*prior_code, None);
            assert!(sqlite_code.is_some());
            assert!(sqlite_extended_code.is_some());
            *run_id
        }
        other => panic!("unexpected result: {other:?}"),
    };
    assert_eq!(error.code(), "data_operation_outcome_unknown");
    assert!(error.user_message().contains(&run_id.to_string()));
    let wire = serde_json::to_value(crate::ai::error::AiCommandError::from(error)).unwrap();
    assert_eq!(wire["code"], "data_operation_outcome_unknown");
    assert!(
        wire["message"]
            .as_str()
            .unwrap()
            .contains(&run_id.to_string())
    );
    assert!(guard.is_autocommit());
    drop(guard);
    assert!(
        db.lock().is_err(),
        "unknown commit blocks further admission"
    );
    drop(db);

    let sql = Connection::open(&sqlite_path).unwrap();
    let reopened = Database::open(sql, &log_path).unwrap();
    let guard = reopened.lock().unwrap();
    let count: i64 = guard
        .query_row(
            "select count(*) from ai_runs where id=?1",
            [run_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "the failed commit was rolled back on this fixture"
    );
}

#[test]
fn guarded_creation_preserves_unknown_when_rollback_fails() {
    use crate::data_interface::Database;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

    let directory = tempfile::tempdir().unwrap();
    let sqlite_path = directory.path().join("data.sqlite");
    let log_path = directory.path().join("log");
    let mut sql = Connection::open(&sqlite_path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let (anchor, _) = insert_selection_source(&sql);
    let credentials = TestDirectory::new();
    let mut draft = selection_draft(&anchor);
    draft.confirmed_scope_hash = Some(
        preview_task(&sql, &credentials.credentials(), &draft)
            .unwrap()
            .scope_hash,
    );
    let db = Database::open(sql, &log_path).unwrap();
    let mut guard = db.lock().unwrap();
    guard
        .execute_batch(
            "create temp trigger fault_scope before insert on main.ai_scope_items
         begin select raise(abort, 'fixture scope failure'); end;",
        )
        .unwrap();
    guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = insert_run_guarded(&mut guard, &credentials.credentials(), &draft, None)
        .expect_err("rollback is denied after the scope write fails");
    assert!(matches!(
        &error,
        AiError::RunCreationUnknown {
            phase: "rollback",
            safe_cause: "sql_rollback_failed",
            prior_code: Some("storage_error"),
            ..
        }
    ));
    assert!(error.user_message().contains("原错误码：storage_error"));
    assert!(!guard.is_autocommit());
    guard.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    guard.execute_batch("rollback").unwrap();
    drop(guard);
    assert!(
        db.lock().is_err(),
        "manual cleanup must not clear the recovery barrier"
    );
    drop(db);

    let sql = Connection::open(&sqlite_path).unwrap();
    let reopened = Database::open(sql, &log_path).unwrap();
    let guard = reopened.lock().unwrap();
    let count: i64 = guard
        .query_row("select count(*) from ai_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn commit_and_cleanup_failure_keep_the_original_sqlite_code() {
    use crate::data_interface::Database;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

    let directory = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(directory.path().join("data.sqlite")).unwrap();
    sql.execute_batch("pragma foreign_keys=on").unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let (anchor, _) = insert_selection_source(&sql);
    let credentials = TestDirectory::new();
    let mut draft = selection_draft(&anchor);
    draft.confirmed_scope_hash = Some(
        preview_task(&sql, &credentials.credentials(), &draft)
            .unwrap()
            .scope_hash,
    );
    let db = Database::open(sql, &directory.path().join("log")).unwrap();
    let mut guard = db.lock().unwrap();
    guard.execute_batch("create temp table fault_parent(id text primary key);
        create temp table fault_child(id text references fault_parent(id) deferrable initially deferred);
        create temp trigger fault_ai_run after insert on main.ai_runs
        begin insert into fault_child values ('missing'); end;").unwrap();
    guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error =
        insert_run_guarded(&mut guard, &credentials.credentials(), &draft, None).unwrap_err();
    assert!(matches!(
        &error,
        AiError::RunCreationUnknown {
            phase: "commit",
            safe_cause: "sql_commit_unknown_cleanup_failed",
            sqlite_extended_code: Some(787),
            ..
        }
    ));
    let message = error.user_message();
    assert!(message.contains("sql_commit_unknown_cleanup_failed"));
    assert!(message.contains("SQLite 扩展码：787"));
    assert!(!message.contains("missing"));
    assert!(!guard.is_autocommit());
    guard.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    guard.execute_batch("rollback").unwrap();
    drop(guard);
    assert!(db.lock().is_err());
}

#[test]
fn guarded_creation_preserves_confirmed_rejection_and_allows_later_creation() {
    use crate::data_interface::Database;

    let directory = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(directory.path().join("data.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let (anchor, _) = insert_selection_source(&sql);
    let credentials = TestDirectory::new();
    let mut draft = selection_draft(&anchor);
    let scope = preview_task(&sql, &credentials.credentials(), &draft)
        .unwrap()
        .scope_hash;
    draft.confirmed_scope_hash = Some("stale".to_owned());
    let db = Database::open(sql, &directory.path().join("log")).unwrap();
    let mut guard = db.lock().unwrap();
    assert!(matches!(
        insert_run_guarded(&mut guard, &credentials.credentials(), &draft, None),
        Err(AiError::InvalidInput(_))
    ));
    drop(guard);
    let mut guard = db.lock().unwrap();
    draft.confirmed_scope_hash = Some(scope);
    let run = insert_run_guarded(&mut guard, &credentials.credentials(), &draft, None).unwrap();
    assert_eq!(run.task_kind, "selection_explanation");
    assert!(run.shared_request_id.is_some());
    assert!(guard.is_autocommit());
}
