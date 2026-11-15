use super::*;
use tokio::net::TcpListener;

#[tokio::test]
async fn initial_window_commit_failure_stops_real_transport_and_owned_turn_before_http() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agent.sqlite");
    let log = root.path().join("log");
    let mut sql = rusqlite::Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after insert on agent_initial_context_windows begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &log).unwrap();
    let app = crate::storage::AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let fixture = Fixture {
        _root: root,
        app,
        service,
        store,
        id,
    };
    let record = fixture.record("original prompt").await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let execution = fixture.execution(&record, fixture.transport(&record, &url));
    let owned = fixture.start(&record, execution, false).await;
    let observed = owned.task.observe().await;
    assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
    let detail = serde_json::to_value(observed.diagnostic.unwrap()).unwrap();
    assert_eq!(detail["conversationId"], id.to_string());
    assert_eq!(detail["phase"], "commit");
    assert_eq!(detail["safeCause"], "sql_commit_outcome_unknown");
    assert_eq!(detail["sqliteExtendedCode"], 787);
    let listener = listener.into_std().unwrap();
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert!(db.lock().is_err());
    drop(owned);
    let Fixture {
        _root: root,
        app,
        service,
        store,
        ..
    } = fixture;
    drop(store);
    drop(service);
    drop(app);
    drop(db);

    let sql = rusqlite::Connection::open(&path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log).unwrap();
    let app = crate::storage::AppStore::for_test_database(reopened, root.path().join("objects"));
    let service = AgentSessionService::from_app_store(&app).unwrap();
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(record.turn_id).unwrap();
    assert_eq!(
        turn.outcome,
        Some(research_agent_core::protocol::TurnOutcome::Interrupted)
    );
    assert!(turn.attempts.is_empty());
}
