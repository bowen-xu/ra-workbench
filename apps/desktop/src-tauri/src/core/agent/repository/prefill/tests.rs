use super::*;

#[test]
fn migration_installs_on_real_schema_and_deletion_cascades() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 129);
    let id = Uuid::from_u128(1);
    crate::agent::repository::create_conversation(
        &mut db,
        research_agent_core::conversation::ConversationId::from_uuid(id),
        "test",
    )
    .unwrap();
    let value = serde_json::to_string(&PrefillBaseline {
        window: None,
        tokens: 100,
        observed: false,
    })
    .unwrap();
    db.execute("insert into agent_prefill_baselines(conversation_id,model_json,window_key,rollback_epoch,baseline_json) values(?1,?2,'',0,?3)", params![id.to_string(),serde_json::to_string(&model("model")).unwrap(),value]).unwrap();
    db.execute(
        "delete from agent_conversations where id=?1",
        [id.to_string()],
    )
    .unwrap();
    let count: u64 = db
        .query_row("select count(*) from agent_prefill_baselines", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

fn model(id: &str) -> ModelIdentity {
    ModelIdentity {
        provider_id: "fixture".into(),
        model_id: id.into(),
        revision: None,
    }
}
fn connection(path: &std::path::Path) -> Arc<Mutex<Connection>> {
    let db = Connection::open(path).unwrap();
    db.execute_batch("pragma foreign_keys=on; create table if not exists agent_conversations(id text primary key, archived integer not null); create table if not exists agent_active_context_windows(conversation_id text primary key, window_id text); create table if not exists agent_items(conversation_id text, ordinal integer, payload_json text);").unwrap();
    let exists: bool = db
        .query_row(
            "select exists(select 1 from sqlite_master where name='agent_prefill_baselines')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if !exists {
        db.execute_batch(include_str!(
            "../../../../../migrations/0129_agent_prefill_baselines.sql"
        ))
        .unwrap();
    }
    Arc::new(Mutex::new(db))
}
fn estimator(db: Arc<Mutex<Connection>>, id: Uuid, name: &str) -> SqlitePrefillEstimator {
    SqlitePrefillEstimator::new(
        db,
        id,
        &model(name),
        Arc::new(ServerObservedEstimator::new(Arc::new(
            Utf8JsonTokenEstimator,
        ))),
    )
    .unwrap()
}

#[tokio::test]
async fn failed_prefill_write_preserves_safe_sql_diagnostic_and_known_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(dir.path().join("prefill.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut sql,
        research_agent_core::conversation::ConversationId::from_uuid(id),
        "test",
    )
    .unwrap();
    sql.execute_batch("create trigger reject_prefill before insert on agent_prefill_baselines begin select raise(abort,'private fixture detail'); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &dir.path().join("log")).unwrap();
    let estimator = SqlitePrefillEstimator::new(
        db.clone(),
        id,
        &model("model"),
        Arc::new(ServerObservedEstimator::new(Arc::new(
            Utf8JsonTokenEstimator,
        ))),
    )
    .unwrap();
    let error = estimator
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id, phase: "write", safe_cause: "sql_write_failed", sqlite_extended_code: Some(1811),
    }) if conversation_id == id)
    );
    assert!(!error.to_string().contains("private fixture detail"));
    let research_agent_core::runner::RunError::EventSinkDiagnostic(detail) = error.into() else {
        panic!("known rollback must retain the safe persistence diagnostic");
    };
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(detail["conversationId"], id.to_string());
    assert_eq!(detail["phase"], "write");
    assert_eq!(detail["safeCause"], "sql_write_failed");
    assert_eq!(detail["sqliteExtendedCode"], 1811);
    {
        let guard = db.lock().expect("confirmed rollback keeps owner available");
        let count: u64 = guard
            .query_row("select count(*) from agent_prefill_baselines", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        guard.execute_batch("drop trigger reject_prefill").unwrap();
    }
    estimator
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_prefill_read_preserves_safe_sql_diagnostic_and_known_rollback() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let dir = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(dir.path().join("prefill.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut sql,
        research_agent_core::conversation::ConversationId::from_uuid(id),
        "test",
    )
    .unwrap();
    let db = crate::data_interface::Database::open(sql, &dir.path().join("log")).unwrap();
    {
        let guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "agent_conversations",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    let estimator = SqlitePrefillEstimator::new(
        db.clone(),
        id,
        &model("model"),
        Arc::new(ServerObservedEstimator::new(Arc::new(
            Utf8JsonTokenEstimator,
        ))),
    )
    .unwrap();
    let error = estimator
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id, phase: "read", safe_cause: "sql_read_failed", sqlite_extended_code: Some(23),
    }) if conversation_id == id)
    );
    {
        let guard = db.lock().expect("confirmed rollback keeps owner available");
        guard.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
    estimator
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_prefill_commit_fences_paired_owner_until_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefill.sqlite");
    let log = dir.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table prefill_parent(id integer primary key); create table prefill_child(id integer references prefill_parent(id) deferrable initially deferred); create trigger prefill_fail after insert on agent_prefill_baselines begin insert into prefill_child values(1); end;").unwrap();
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut sql,
        research_agent_core::conversation::ConversationId::from_uuid(id),
        "test",
    )
    .unwrap();
    let db = crate::data_interface::Database::open(sql, &log).unwrap();
    let estimator = SqlitePrefillEstimator::new(
        db.clone(),
        id,
        &model("model"),
        Arc::new(ServerObservedEstimator::new(Arc::new(
            Utf8JsonTokenEstimator,
        ))),
    )
    .unwrap();
    assert!(matches!(
        estimator.prepare_auto_compaction(Some(100), None).await,
        Err(ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "commit", safe_cause: "sql_commit_outcome_unknown",
            sqlite_extended_code: Some(787),
        })) if conversation_id == id
    ));
    assert!(db.lock().is_err(), "uncertain commit must fence the owner");
    drop(estimator);
    drop(db);
    let sql = Connection::open(&path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log).unwrap();
    let guard = reopened.lock().unwrap();
    let count: u64 = guard
        .query_row("select count(*) from agent_prefill_baselines", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn failed_prefill_rollback_fences_owner_but_known_rejection_does_not() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let dir = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(dir.path().join("prefill.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &dir.path().join("log")).unwrap();
    let id = Uuid::new_v4();
    let estimator = SqlitePrefillEstimator::new(
        db.clone(),
        id,
        &model("model"),
        Arc::new(ServerObservedEstimator::new(Arc::new(
            Utf8JsonTokenEstimator,
        ))),
    )
    .unwrap();
    assert!(matches!(
        estimator.prepare_auto_compaction(Some(100), None).await,
        Err(ModelError::InvalidRequest)
    ));
    {
        let guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    assert!(matches!(
        estimator.prepare_auto_compaction(Some(100), None).await,
        Err(ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "rollback", safe_cause: "sql_rollback_failed", ..
        })) if conversation_id == id
    ));
    assert!(db.lock().is_err(), "failed rollback must fence the owner");
}

#[tokio::test]
async fn repeated_identical_baseline_does_not_rewrite_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let db = connection(&dir.path().join("prefill-noop.sqlite"));
    let id = Uuid::new_v4();
    {
        let sql = db.lock().unwrap();
        sql.execute(
            "insert into agent_conversations(id,archived) values(?1,0)",
            [id.to_string()],
        )
        .unwrap();
        sql.execute_batch("create temp table baseline_update_count(n integer not null); insert into baseline_update_count values(0); create temp trigger count_baseline_update after update on main.agent_prefill_baselines begin update baseline_update_count set n=n+1; end;").unwrap();
    }
    let estimator = estimator(db.clone(), id, "model");
    estimator
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap();
    estimator
        .prepare_auto_compaction(Some(200), None)
        .await
        .unwrap();
    let updates: u64 = db
        .lock()
        .unwrap()
        .query_row("select n from baseline_update_count", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        updates, 0,
        "identical prepared baseline should not rewrite the stored row"
    );
    estimator
        .observe(
            &[],
            &[],
            ModelUsage {
                input_tokens: Some(90),
                output_tokens: Some(20),
            },
            None,
        )
        .unwrap();
    estimator.persist_auto_compaction(None).await.unwrap();
    estimator.persist_auto_compaction(None).await.unwrap();
    let updates: u64 = db
        .lock()
        .unwrap()
        .query_row("select n from baseline_update_count", [], |row| row.get(0))
        .unwrap();
    assert_eq!(updates, 1, "first observation must be saved exactly once");
}

#[tokio::test]
async fn reopen_preserves_first_input_baseline_and_exact_model_scope() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefill.sqlite");
    let id = Uuid::from_u128(1);
    let db = connection(&path);
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,archived) values(?1,0)",
            [id.to_string()],
        )
        .unwrap();
    let first = estimator(db.clone(), id, "model");
    first
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap();
    first
        .observe(
            &[],
            &[],
            ModelUsage {
                input_tokens: Some(90),
                output_tokens: Some(20),
            },
            None,
        )
        .unwrap();
    first.persist_auto_compaction(None).await.unwrap();
    first
        .observe(
            &[],
            &[],
            ModelUsage {
                input_tokens: Some(150),
                output_tokens: Some(20),
            },
            None,
        )
        .unwrap();
    first.persist_auto_compaction(None).await.unwrap();
    drop(first);
    drop(db);
    let db = connection(&path);
    let resumed = estimator(db.clone(), id, "model");
    resumed
        .prepare_auto_compaction(Some(200), None)
        .await
        .unwrap();
    assert_eq!(
        resumed.auto_compaction_tokens(Some(200), None).unwrap(),
        Some(110)
    );
    let alias = estimator(db, id, "alias");
    alias
        .prepare_auto_compaction(Some(200), None)
        .await
        .unwrap();
    assert_eq!(
        alias.auto_compaction_tokens(Some(200), None).unwrap(),
        Some(0)
    );
}

#[tokio::test]
async fn rollback_fences_observation_and_resets_root_window_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let db = connection(&dir.path().join("rollback.sqlite"));
    let id = Uuid::from_u128(1);
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,archived) values(?1,0)",
            [id.to_string()],
        )
        .unwrap();
    let stale = estimator(db.clone(), id, "model");
    stale
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap();
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_items(conversation_id,ordinal,payload_json) values(?1,10,?2)",
            params![
                id.to_string(),
                r#"{"payload":{"type":"conversationRolledBack"}}"#
            ],
        )
        .unwrap();
    stale
        .observe(
            &[],
            &[],
            ModelUsage {
                input_tokens: Some(90),
                output_tokens: Some(20),
            },
            None,
        )
        .unwrap();
    assert_eq!(
        stale.persist_auto_compaction(None).await,
        Err(ModelError::InvalidRequest)
    );
    let current = estimator(db.clone(), id, "model");
    current
        .prepare_auto_compaction(Some(40), None)
        .await
        .unwrap();
    assert_eq!(
        current.auto_compaction_tokens(Some(45), None).unwrap(),
        Some(5)
    );
    db.lock()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [id.to_string()],
        )
        .unwrap();
    assert_eq!(
        current.prepare_auto_compaction(Some(50), None).await,
        Err(ModelError::InvalidRequest)
    );
}

#[tokio::test]
async fn changed_active_window_and_corrupt_state_fail_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let db = connection(&dir.path().join("window.sqlite"));
    let id = Uuid::from_u128(1);
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,archived) values(?1,0)",
            [id.to_string()],
        )
        .unwrap();
    let current = estimator(db.clone(), id, "model");
    current
        .prepare_auto_compaction(Some(100), None)
        .await
        .unwrap();
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_active_context_windows(conversation_id,window_id) values(?1,?2)",
            params![id.to_string(), Uuid::from_u128(2).to_string()],
        )
        .unwrap();
    assert_eq!(
        current.persist_auto_compaction(None).await,
        Err(ModelError::InvalidRequest)
    );
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_active_context_windows where conversation_id=?1",
            [id.to_string()],
        )
        .unwrap();
    db.lock()
        .unwrap()
        .execute("update agent_prefill_baselines set baseline_json='{}'", [])
        .unwrap();
    assert_eq!(
        current.prepare_auto_compaction(Some(40), None).await,
        Err(ModelError::InvalidStream)
    );
    let saved: String = db
        .lock()
        .unwrap()
        .query_row(
            "select baseline_json from agent_prefill_baselines",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, "{}");
}
