use super::SqliteConversationStore;
use research_agent_core::conversation::{
    ConversationId, ConversationStore, HistoryQuery, LiveConversation, StoreError, WriterId,
    WriterLease,
};
use rusqlite::Connection;
use std::{
    future::Future,
    sync::{Arc, Mutex},
    task::{Context, Waker},
};
use uuid::Uuid;

fn lease(id: ConversationId) -> WriterLease {
    WriterLease {
        conversation_id: id,
        writer_id: WriterId::from_uuid(Uuid::new_v4()),
    }
}

fn setup() -> (
    Arc<Mutex<Connection>>,
    Arc<SqliteConversationStore<String>>,
    ConversationId,
) {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 67);
    let connection = Arc::new(Mutex::new(connection));
    let store =
        Arc::new(SqliteConversationStore::new(Arc::clone(&connection), "Paper".into()).unwrap());
    (connection, store, ConversationId::from_uuid(Uuid::new_v4()))
}

#[tokio::test]
async fn live_owner_uses_lazy_sqlite_storage_and_preserves_committed_prefix() {
    let (_, store, id) = setup();
    let live = LiveConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    let query = HistoryQuery::new(id, 0, 10).unwrap();
    assert_eq!(
        store.history(query).await,
        Err(StoreError::NotFound {
            conversation_id: id
        })
    );
    live.append(&["first".into()]).await.unwrap();
    assert_eq!(live.persist().await, Ok(1));
    live.append(&["unsaved".into()]).await.unwrap();
    live.discard();
    let (resumed, page) = LiveConversation::resume(store.clone(), lease(id), 10)
        .await
        .unwrap();
    assert_eq!(page.items, ["first"]);
    resumed.append(&["second".into()]).await.unwrap();
    assert_eq!(resumed.shutdown().await.ok(), Some(2));
    assert_eq!(
        store.history(query).await.unwrap().items,
        ["first", "second"]
    );
}

#[tokio::test]
async fn failed_first_commit_does_not_materialize_metadata_and_can_retry() {
    let (connection, store, id) = setup();
    let live = LiveConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    connection.lock().unwrap().execute_batch(
        "create trigger fail_item before insert on agent_items begin select raise(abort, 'injected'); end;"
    ).unwrap();
    live.append(&["pending".into()]).await.unwrap();
    assert_eq!(live.persist().await, Err(StoreError::Unavailable));
    assert_eq!(
        store.history(HistoryQuery::new(id, 0, 10).unwrap()).await,
        Err(StoreError::NotFound {
            conversation_id: id
        })
    );
    connection
        .lock()
        .unwrap()
        .execute_batch("drop trigger fail_item;")
        .unwrap();
    assert_eq!(live.persist().await, Ok(1));
    assert_eq!(live.flush().await, Ok(1));
}

#[tokio::test]
async fn empty_persist_is_durable_and_competing_or_stale_handles_do_not_close_it() {
    let (_, store, id) = setup();
    let old_lease = lease(id);
    let old = LiveConversation::create(store.clone(), old_lease)
        .await
        .unwrap();
    assert_eq!(old.persist().await, Ok(0));
    assert!(matches!(
        LiveConversation::resume(store.clone(), lease(id), 10).await,
        Err(StoreError::WriterConflict)
    ));
    store.discard(old_lease);
    let (current, page) = LiveConversation::resume(store.clone(), lease(id), 10)
        .await
        .unwrap();
    assert!(page.items.is_empty());
    drop(old);
    current.append(&["safe".into()]).await.unwrap();
    assert_eq!(current.flush().await, Ok(1));
}

#[test]
fn dropping_queued_flush_or_owner_cancels_before_database_commit() {
    for discard_owner in [false, true] {
        let (connection, store, id) = setup();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let retained_owner = runtime.block_on(async {
            let live = LiveConversation::create(store.clone(), lease(id))
                .await
                .unwrap();
            live.append(&["must not commit".into()]).await.unwrap();
            // Hold the database so the actual blocking job cannot obtain commit permission.
            let database_guard = connection.lock().unwrap();
            let mut flush = Box::pin(live.flush());
            let mut context = Context::from_waker(Waker::noop());
            assert!(flush.as_mut().poll(&mut context).is_pending());
            drop(flush);
            drop(database_guard);
            if discard_owner {
                live.discard();
                None
            } else {
                Some(live)
            }
        });
        // Runtime shutdown joins all blocking work; no timed sleep or guessed completion.
        drop(runtime);
        let count: i64 = connection
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_conversations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        drop(retained_owner);
    }
}

#[tokio::test]
async fn typed_events_survive_storage_and_rebuild_the_conversation() {
    use research_agent_core::protocol::{AgentEvent, ConversationProjection, TurnOutcome};
    let (connection, _, _) = setup();
    let events: Vec<AgentEvent> = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/agent-events-v1.json"
    )))
    .unwrap();
    let id = events[0].conversation_id();
    let store = Arc::new(
        SqliteConversationStore::<AgentEvent>::new(connection, "Protocol fixture".into()).unwrap(),
    );
    let live = LiveConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    live.append(&events).await.unwrap();
    assert_eq!(live.shutdown().await.ok(), Some(events.len() as u64));
    let (_resumed, page) = LiveConversation::resume(store, lease(id), 100)
        .await
        .unwrap();
    assert_eq!(page.items, events);
    let mut projection = ConversationProjection::new(id);
    for event in page.items {
        projection.apply(&event).unwrap();
    }
    assert_eq!(
        projection.turn(Uuid::from_u128(2)).unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
}

#[tokio::test]
async fn pending_limit_rejection_does_not_partially_append() {
    let (_, store, id) = setup();
    let live = LiveConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    live.append(&vec!["kept".into(); 499]).await.unwrap();
    assert_eq!(
        live.append(&["no".into(), "no".into()]).await,
        Err(StoreError::InvalidBatch)
    );
    assert_eq!(live.persist().await, Ok(499));
}
