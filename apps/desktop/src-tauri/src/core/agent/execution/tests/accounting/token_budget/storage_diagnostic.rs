use super::*;

#[tokio::test]
async fn initial_window_commit_failure_preserves_diagnostic_with_token_budget_enabled() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agent.sqlite");
    let log = root.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after insert on agent_initial_context_windows begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &log).unwrap();
    let app = crate::storage::AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let ((service, store, transport, tools, mut execution, id), _) = setup_with_app(&app).await;
    buffered(&mut execution, transport.clone(), 40, false);
    let record = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
    let admission = Arc::new(execution)
        .start(
            service.clone(),
            store,
            ConversationId::from_uuid(id),
            record.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("expected started task")
    };
    let observed = owned.task.observe().await;
    assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
    let detail = serde_json::to_value(observed.diagnostic.unwrap()).unwrap();
    assert_eq!(detail["conversationId"], id.to_string());
    assert_eq!(detail["phase"], "commit");
    assert_eq!(detail["safeCause"], "sql_commit_outcome_unknown");
    assert_eq!(detail["sqliteExtendedCode"], 787);
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    assert!(db.lock().is_err());
}

#[tokio::test]
async fn fragment_commit_failure_stops_followup_and_retains_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("agent.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after insert on agent_context_fragments begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let app = crate::storage::AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let ((service, store, transport, tools, mut execution, id), _) = setup_with_app(&app).await;
    buffered(&mut execution, transport.clone(), 120, true);
    let record = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
    let admission = Arc::new(execution)
        .start(
            service.clone(),
            store,
            ConversationId::from_uuid(id),
            record.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new task expected")
    };
    let observed = owned.task.observe().await;
    assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
    let detail = serde_json::to_value(observed.diagnostic.unwrap()).unwrap();
    assert_eq!(detail["conversationId"], id.to_string());
    assert_eq!(detail["phase"], "commit");
    assert_eq!(detail["safeCause"], "sql_commit_outcome_unknown");
    assert_eq!(detail["sqliteExtendedCode"], 787);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    assert!(db.lock().is_err());
}
