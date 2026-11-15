//! Recreate both the service and file-backed connection, without a graceful shutdown.
use super::*;
use research_agent_core::{protocol::TurnOutcome, steering::SteeringRecoveryStore};
use std::path::Path;

fn reopen(path: &Path) -> (AgentSessionService, SqliteSteeringStore) {
    let mut connection = Connection::open(path).unwrap();
    // Reopen the deployed schema through normal initialization, including the
    // scalar functions required by journal identity triggers after migration 211.
    crate::storage::migrate(&mut connection).unwrap();
    let db = Arc::new(Mutex::new(connection));
    (
        AgentSessionService::new(Arc::new(
            SqliteConversationStore::new(db.clone(), "restart".into()).unwrap(),
        )),
        SqliteSteeringStore::new(db),
    )
}

#[tokio::test]
async fn abrupt_restart_retains_accepted_inputs_and_never_admits_a_new_turn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("steering.sqlite3");
    let (db, service, store, scope) = setup_connection(Connection::open(&path).unwrap()).await;
    let first = input(scope);
    let second = SteeringInput {
        text: "second accepted input".into(),
        ..input(scope)
    };
    store.enqueue(first.clone()).await.unwrap();
    store.enqueue(second.clone()).await.unwrap();
    // Simulate loss of the UI replies and then process lifetime. No finish or shutdown call.
    drop(store);
    drop(service);
    drop(db);

    let (service, store) = reopen(&path);
    let id = ConversationId::from_uuid(scope.conversation_id);
    assert_eq!(store.recoverable(scope).await, Err(SteeringError::Closed));
    let mut notices = service.subscribe();
    let history = service.open(id).await.unwrap();
    assert_eq!(history.active_turn(), None);
    assert_eq!(
        history.turn(scope.turn_id).unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(history.seq(), 2);
    assert_eq!(notices.try_recv().unwrap().watermark, 2);
    for _ in 0..2 {
        assert_eq!(
            store.recoverable(scope).await.unwrap(),
            vec![first.clone(), second.clone()]
        );
        assert_eq!(service.open(id).await.unwrap().seq(), 2);
    }
    assert!(notices.try_recv().is_err());
    // Original receipt replay is a read of the previous admission, not a new input.
    store.enqueue(first.clone()).await.unwrap();
    assert_eq!(
        store.enqueue(input(scope)).await,
        Err(SteeringError::Closed)
    );
    assert_eq!(service.read_since(id, 0, 20).await.unwrap().items.len(), 2);
    assert!(
        SessionTurnInput {
            evidence_store: None,
            injected: std::sync::Mutex::default(),
            service: &service,
            store: &store,
            scope
        }
        .drain(scope.turn_id, false)
        .await
        .is_err()
    );
    drop(store);
    drop(service);
    let (service, store) = reopen(&path);
    assert_eq!(service.open(id).await.unwrap().seq(), 2);
    assert_eq!(store.recoverable(scope).await.unwrap(), vec![first, second]);
}

#[tokio::test]
async fn restart_preserves_consumed_history_without_replaying_it_to_a_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("consumed.sqlite3");
    let (db, service, store, scope) = setup_connection(Connection::open(&path).unwrap()).await;
    let consumed = input(scope);
    store.enqueue(consumed.clone()).await.unwrap();
    let messages = SessionTurnInput {
        evidence_store: None,
        injected: std::sync::Mutex::default(),
        service: &service,
        store: &store,
        scope,
    }
    .drain(scope.turn_id, false)
    .await
    .unwrap();
    assert_eq!(messages.len(), 1);
    // The model has not received these messages. Recovery must preserve history without
    // guessing whether an external dispatch succeeded or silently paying to replay it.
    drop(messages);
    let pending = SteeringInput {
        text: "accepted after the boundary".into(),
        ..input(scope)
    };
    store.enqueue(pending.clone()).await.unwrap();
    drop(store);
    drop(service);
    drop(db);

    let (service, store) = reopen(&path);
    let id = ConversationId::from_uuid(scope.conversation_id);
    let history = service.open(id).await.unwrap();
    let turn = history.turn(scope.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Interrupted));
    assert_eq!(turn.items.len(), 1);
    assert!(turn.items[&consumed.input_id].completed);
    assert_eq!(turn.items[&consumed.input_id].text, consumed.text);
    assert_eq!(history.seq(), 5);
    assert_eq!(store.recoverable(scope).await.unwrap(), vec![pending]);
    store.enqueue(consumed).await.unwrap();
    assert_eq!(service.open(id).await.unwrap().seq(), 5);
}

#[tokio::test]
async fn crash_after_failed_consumption_commit_recovers_all_accepted_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("failed-commit.sqlite3");
    let (db, service, store, scope) = setup_connection(Connection::open(&path).unwrap()).await;
    let accepted = input(scope);
    store.enqueue(accepted.clone()).await.unwrap();
    // Fail after the batch has begun inserting, proving rollback of the whole user item.
    db.lock().unwrap().execute_batch("create trigger reject_delta before insert on agent_journal_records when json_extract(new.payload_json,'$.payload.type')='textDelta' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        SessionTurnInput {
            evidence_store: None,
            injected: std::sync::Mutex::default(),
            service: &service,
            store: &store,
            scope
        }
        .drain(scope.turn_id, false)
        .await
        .is_err()
    );
    drop(store);
    drop(service);
    drop(db);

    let (service, store) = reopen(&path);
    let history = service
        .open(ConversationId::from_uuid(scope.conversation_id))
        .await
        .unwrap();
    assert_eq!(history.seq(), 2);
    assert!(history.turn(scope.turn_id).unwrap().items.is_empty());
    assert_eq!(
        history.turn(scope.turn_id).unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(store.recoverable(scope).await.unwrap(), vec![accepted]);
}
