use super::*;

#[tokio::test]
async fn unknown_window_install_stops_before_post_compact_and_retains_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("agent.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after insert on agent_compact_session_start begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let app = crate::storage::AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let ((service, store, transport, _, mut execution, id), _) = setup_with_app(&app).await;
    let hooks = Arc::new(Hooks {
        stop_at: None,
        events: Mutex::new(vec![]),
    });
    execution.compaction_hooks = Some(hooks.clone());
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let (_, admission) = compact(execution, service, store, id).await;
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
    let hooks = hooks.events.lock().unwrap();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].stage, CompactionHookStage::PreCompact);
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
    assert!(db.lock().is_err());
}

#[tokio::test]
async fn unknown_hook_consumption_stops_dispatch_and_preserves_task_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("agent.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred);").unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let app = crate::storage::AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let ((service, store, transport, _, mut execution, id), _) = setup_with_app(&app).await;
    let hooks = Arc::new(Hooks {
        stop_at: None,
        events: Mutex::new(vec![]),
    });
    execution.compaction_hooks = Some(hooks.clone());
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let old_hooks = hooks.events.lock().unwrap().len();
    assert_eq!(old_hooks, 2);
    let old_requests = transport.requests.lock().unwrap().len();
    {
        let guard = db.lock().unwrap();
        guard.execute_batch("create trigger fixture_fail after delete on agent_compact_session_start begin insert into fixture_child values(1); end;").unwrap();
    }
    let next = store
        .put(id, Uuid::new_v4(), input("follow-up"))
        .await
        .unwrap();
    let admission = execution
        .start(
            service.clone(),
            store,
            ConversationId::from_uuid(id),
            next.request_id,
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
    assert_eq!(hooks.events.lock().unwrap().len(), old_hooks);
    assert_eq!(transport.requests.lock().unwrap().len(), old_requests);
    assert!(db.lock().is_err());
}
