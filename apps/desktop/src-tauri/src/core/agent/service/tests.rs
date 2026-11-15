use super::super::repository::SqliteConversationStore;
use super::*;
use research_agent_core::conversation::{
    ConversationStore, HistoryPage, HistoryQuery, StoreFuture, StoreResult, WriterLease,
};
use research_agent_core::protocol::{EventPayload, ModelIdentity, TurnOutcome, TurnPhase};
use rusqlite::Connection;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

#[path = "tests/token_budget_window.rs"]
mod token_budget_window;

pub(super) fn setup() -> (
    Arc<Mutex<Connection>>,
    Arc<SqliteConversationStore<AgentEvent>>,
    AgentSessionService,
    ConversationId,
) {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 67);
    let connection = Arc::new(Mutex::new(connection));
    let store =
        Arc::new(SqliteConversationStore::new(connection.clone(), "Service".into()).unwrap());
    let service = AgentSessionService::new(store.clone());
    (
        connection,
        store,
        service,
        ConversationId::from_uuid(Uuid::new_v4()),
    )
}
fn event(id: ConversationId, seq: u64, payload: EventPayload) -> AgentEvent {
    AgentEvent::new(id, seq, 0, Uuid::from_u128(4), payload).unwrap()
}
fn start(id: ConversationId) -> AgentEvent {
    event(
        id,
        1,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: Uuid::from_u128(2),
            context_snapshot_id: Uuid::from_u128(3),
            model: ModelIdentity {
                provider_id: "test".into(),
                model_id: "test".into(),
                revision: None,
            },
        },
    )
}

#[tokio::test]
async fn hosts_share_live_session_and_notifications_follow_durable_commits() {
    let (_, _, service, id) = setup();
    let mut notices = service.subscribe();
    service.create(id).await.unwrap();
    service.commit(id, vec![start(id)]).await.unwrap();
    assert_eq!(
        notices.try_recv().unwrap(),
        CommitNotice {
            conversation_id: id,
            watermark: 1
        }
    );
    let page = service.read_since(id, 0, 10).await.unwrap();
    assert_eq!(page.items, [start(id)]);
    let second_host = service.open(id).await.unwrap();
    assert_eq!(second_host.active_turn(), Some(Uuid::from_u128(2)));
    assert!(notices.try_recv().is_err());
    assert!(matches!(
        service.release_idle(id).await,
        Err(AgentServiceError::Active)
    ));
    service
        .commit(
            id,
            vec![event(
                id,
                2,
                EventPayload::TurnFinished {
                    turn_id: Uuid::from_u128(2),
                    outcome: TurnOutcome::Completed,
                },
            )],
        )
        .await
        .unwrap();
    service.release_idle(id).await.unwrap();
    assert_eq!(service.open(id).await.unwrap().seq(), 2);
}

#[tokio::test]
async fn opening_history_does_not_wait_for_a_live_writer() {
    let (_, _, service, id) = setup();
    service.create(id).await.unwrap();
    service.commit(id, vec![start(id)]).await.unwrap();

    let slot = service.slot(id, false).unwrap();
    let _writer = slot.lock().await;
    let opened = tokio::time::timeout(std::time::Duration::from_millis(100), service.open(id))
        .await
        .expect("history open must not wait for the active writer")
        .unwrap();
    assert_eq!(opened.seq(), 1);
    assert_eq!(opened.active_turn(), Some(Uuid::from_u128(2)));
}

struct GatedHistoryStore {
    inner: Arc<SqliteConversationStore<AgentEvent>>,
    first_page_ready: Arc<Notify>,
    release_first_page: Arc<Notify>,
    history_calls: AtomicUsize,
}

impl ConversationStore<AgentEvent> for GatedHistoryStore {
    fn reserve_writer(&self, lease: WriterLease) -> StoreResult<()> {
        self.inner.reserve_writer(lease)
    }

    fn create(&self, lease: WriterLease) -> StoreFuture<'_, ()> {
        self.inner.create(lease)
    }

    fn resume(&self, lease: WriterLease) -> StoreFuture<'_, ()> {
        self.inner.resume(lease)
    }

    fn append(&self, lease: WriterLease, items: Vec<AgentEvent>) -> StoreFuture<'_, ()> {
        self.inner.append(lease, items)
    }

    fn persist(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
        self.inner.persist(lease)
    }

    fn flush(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
        self.inner.flush(lease)
    }

    fn shutdown(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
        self.inner.shutdown(lease)
    }

    fn discard(&self, lease: WriterLease) {
        self.inner.discard(lease);
    }

    fn history(&self, query: HistoryQuery) -> StoreFuture<'_, HistoryPage<AgentEvent>> {
        let inner = Arc::clone(&self.inner);
        let first_page_ready = Arc::clone(&self.first_page_ready);
        let release_first_page = Arc::clone(&self.release_first_page);
        let first = self.history_calls.fetch_add(1, Ordering::SeqCst) == 0;
        Box::pin(async move {
            let page = inner.history(query).await?;
            if first {
                first_page_ready.notify_one();
                release_first_page.notified().await;
            }
            Ok(page)
        })
    }
}

#[tokio::test]
async fn reopening_a_live_multi_page_history_tolerates_new_commits_between_pages() {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 67);
    let connection = Arc::new(Mutex::new(connection));
    let inner = Arc::new(SqliteConversationStore::new(connection, "Service".into()).unwrap());
    let first_page_ready = Arc::new(Notify::new());
    let release_first_page = Arc::new(Notify::new());
    let store = Arc::new(GatedHistoryStore {
        inner: Arc::clone(&inner),
        first_page_ready: Arc::clone(&first_page_ready),
        release_first_page: Arc::clone(&release_first_page),
        history_calls: AtomicUsize::new(0),
    });
    let service = Arc::new(AgentSessionService::new(store));
    let id = ConversationId::from_uuid(Uuid::new_v4());
    service.create(id).await.unwrap();
    service.commit(id, vec![start(id)]).await.unwrap();
    for seq in 2..=501 {
        service
            .commit(
                id,
                vec![event(
                    id,
                    seq,
                    EventPayload::PhaseChanged {
                        turn_id: Uuid::from_u128(2),
                        phase: TurnPhase::RunningModel,
                    },
                )],
            )
            .await
            .unwrap();
    }

    let slot = service.slot(id, false).unwrap();
    let writer = slot.lock().await;
    let opening = {
        let service = Arc::clone(&service);
        tokio::spawn(async move { service.open(id).await })
    };
    first_page_ready.notified().await;
    drop(writer);
    service
        .commit(
            id,
            vec![event(
                id,
                502,
                EventPayload::PhaseChanged {
                    turn_id: Uuid::from_u128(2),
                    phase: TurnPhase::RunningModel,
                },
            )],
        )
        .await
        .unwrap();
    release_first_page.notify_one();

    let opened = opening.await.unwrap().unwrap();
    assert_eq!(opened.seq(), 501);
    assert_eq!(opened.active_turn(), Some(Uuid::from_u128(2)));
}

#[tokio::test]
async fn service_recreation_recovers_an_active_turn_once() {
    let (_, store, service, id) = setup();
    service.create(id).await.unwrap();
    service.commit(id, vec![start(id)]).await.unwrap();
    drop(service);
    let service = AgentSessionService::new(store);
    let mut notices = service.subscribe();
    let recovered = service.open(id).await.unwrap();
    assert_eq!(
        recovered.turn(Uuid::from_u128(2)).unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(notices.try_recv().unwrap().watermark, 2);
    service.open(id).await.unwrap();
    assert!(notices.try_recv().is_err());
    assert_eq!(service.read_since(id, 0, 10).await.unwrap().items.len(), 2);
}

#[tokio::test]
async fn file_backed_service_reopen_recovers_without_replaying_the_turn() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.sqlite");
    let id = ConversationId::from_uuid(Uuid::new_v4());

    {
        let mut connection = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut connection, 67);
        let database = Arc::new(Mutex::new(connection));
        let store = Arc::new(SqliteConversationStore::new(database, "Service".into()).unwrap());
        let service = AgentSessionService::new(store);
        service.create(id).await.unwrap();
        service.commit(id, vec![start(id)]).await.unwrap();
    }

    let reopened_connection = Arc::new(Mutex::new(Connection::open(&path).unwrap()));
    let reopened_store =
        Arc::new(SqliteConversationStore::new(reopened_connection, "Service".into()).unwrap());
    let reopened = AgentSessionService::new(reopened_store);
    let recovered = reopened.open(id).await.unwrap();
    assert_eq!(
        recovered.turn(Uuid::from_u128(2)).unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(reopened.read_since(id, 0, 10).await.unwrap().items.len(), 2);
    reopened.open(id).await.unwrap();
    assert_eq!(reopened.read_since(id, 0, 10).await.unwrap().items.len(), 2);
}

#[tokio::test]
async fn failed_commit_emits_no_notice_and_requires_explicit_recovery() {
    let (connection, _, service, id) = setup();
    service.create(id).await.unwrap();
    let mut notices = service.subscribe();
    connection.lock().unwrap().execute_batch("create trigger fail_service before insert on agent_items begin select raise(abort, 'injected'); end;").unwrap();
    assert!(service.commit(id, vec![start(id)]).await.is_err());
    assert!(notices.try_recv().is_err());
    assert!(matches!(
        service.open(id).await,
        Err(AgentServiceError::Session(SessionError::RecoveryRequired))
    ));
    connection
        .lock()
        .unwrap()
        .execute_batch("drop trigger fail_service;")
        .unwrap();
    assert_eq!(service.recover(id).await.unwrap().seq(), 0);
    service.commit(id, vec![start(id)]).await.unwrap();
    assert_eq!(notices.try_recv().unwrap().watermark, 1);
}

#[tokio::test]
async fn lagged_notifications_are_recovered_from_ordered_history() {
    let (_, _, service, id) = setup();
    service.create(id).await.unwrap();
    let mut notices = service.subscribe();
    service.commit(id, vec![start(id)]).await.unwrap();
    for seq in 2..=140 {
        service
            .commit(
                id,
                vec![event(
                    id,
                    seq,
                    EventPayload::PhaseChanged {
                        turn_id: Uuid::from_u128(2),
                        phase: TurnPhase::RunningModel,
                    },
                )],
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        notices.try_recv(),
        Err(broadcast::error::TryRecvError::Lagged(_))
    ));
    let page = service.read_since(id, 100, 20).await.unwrap();
    assert_eq!(page.items.first().unwrap().seq(), 101);
    assert_eq!(page.items.last().unwrap().seq(), 120);
    assert_eq!(page.next_offset, Some(120));
    assert_eq!(page.durable_count, 140);
}

#[tokio::test]
async fn different_conversations_keep_independent_owners_and_sequences() {
    let (_, _, service, first) = setup();
    let second = ConversationId::from_uuid(Uuid::new_v4());
    let (a, b) = tokio::join!(service.create(first), service.create(second));
    a.unwrap();
    b.unwrap();
    let (a, b) = tokio::join!(
        service.commit(first, vec![start(first)]),
        service.commit(second, vec![start(second)])
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(
        service.read_since(first, 0, 10).await.unwrap().items,
        [start(first)]
    );
    assert_eq!(
        service.read_since(second, 0, 10).await.unwrap().items,
        [start(second)]
    );
}

#[tokio::test]
async fn runner_sink_allocates_sequences_inside_the_commit_lock() {
    use crate::agent::turn_sink::ServiceTurnSink;
    use research_agent_core::runner::TurnEventSink;
    let (_, _, service, id) = setup();
    service.create(id).await.unwrap();
    service.commit(id, vec![start(id)]).await.unwrap();
    let sink = ServiceTurnSink {
        service: &service,
        conversation_id: id,
        request_id: Uuid::new_v4(),
    };
    let phase = EventPayload::PhaseChanged {
        turn_id: Uuid::from_u128(2),
        phase: TurnPhase::RunningModel,
    };
    let (first, second) = tokio::join!(sink.emit(phase.clone()), sink.emit(phase));
    first.unwrap();
    second.unwrap();
    let page = service.read_since(id, 1, 10).await.unwrap();
    assert_eq!(
        page.items.iter().map(AgentEvent::seq).collect::<Vec<_>>(),
        [2, 3]
    );
}

#[tokio::test]
async fn input_batch_commits_as_one_notice_and_invalid_tail_has_no_partial_effect() {
    use research_agent_core::protocol::ItemKind;
    let (_, _, service, id) = setup();
    service.create(id).await.unwrap();
    let mut notices = service.subscribe();
    let turn = Uuid::from_u128(2);
    let item = Uuid::new_v4();
    let payloads = vec![
        start(id).payload().clone(),
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: item,
            kind: ItemKind::UserMessage,
        },
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: item,
            text: "Revise this paragraph".into(),
        },
        EventPayload::ItemCompleted {
            turn_id: turn,
            item_id: item,
        },
    ];
    let mut invalid = payloads.clone();
    invalid.push(EventPayload::ItemCompleted {
        turn_id: turn,
        item_id: Uuid::new_v4(),
    });
    assert!(
        service
            .commit_payloads(id, Uuid::new_v4(), invalid)
            .await
            .is_err()
    );
    assert_eq!(service.open(id).await.unwrap().seq(), 0);
    assert!(notices.try_recv().is_err());
    assert!(
        service
            .read_since(id, 0, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let receipt = service
        .commit_payloads(id, Uuid::new_v4(), payloads)
        .await
        .unwrap();
    assert_eq!(receipt.watermark, 4);
    assert_eq!(
        receipt
            .events
            .iter()
            .map(AgentEvent::seq)
            .collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(notices.try_recv().unwrap().watermark, 4);
    assert!(notices.try_recv().is_err());
    assert_eq!(
        service.read_since(id, 0, 10).await.unwrap().items,
        receipt.events
    );
}

#[tokio::test]
async fn database_failure_during_input_batch_rolls_back_the_entire_submission() {
    let (connection, _, service, id) = setup();
    service.create(id).await.unwrap();
    let mut notices = service.subscribe();
    connection.lock().unwrap().execute_batch("create trigger fail_batch before insert on agent_items when new.ordinal = 1 begin select raise(abort, 'injected'); end;").unwrap();
    let result = service
        .commit_payloads(
            id,
            Uuid::new_v4(),
            vec![
                start(id).payload().clone(),
                EventPayload::PhaseChanged {
                    turn_id: Uuid::from_u128(2),
                    phase: TurnPhase::RunningModel,
                },
            ],
        )
        .await;
    assert!(result.is_err());
    assert!(
        service
            .read_since(id, 0, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(notices.try_recv().is_err());
    connection
        .lock()
        .unwrap()
        .execute_batch("drop trigger fail_batch;")
        .unwrap();
    assert_eq!(service.recover(id).await.unwrap().seq(), 0);
}
