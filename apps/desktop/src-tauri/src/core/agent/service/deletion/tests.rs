use super::*;
use crate::agent::repository::{SqliteConversationCatalog, SqliteConversationStore};
use research_agent_core::conversation::{
    BindingKind, ConversationBinding, ConversationDeletionFuture,
};
use research_agent_core::protocol::{EventPayload, ModelIdentity};
use rusqlite::Connection;
use tokio::sync::Notify;

fn request() -> ConversationDeletionRequest {
    ConversationDeletionRequest {
        request_id: Uuid::from_u128(2),
        conversation_id: ConversationId::from_uuid(Uuid::from_u128(1)),
        binding: Some(ConversationBinding {
            kind: BindingKind::Manuscript,
            resource_id: "m".into(),
        }),
        expected_title: "Delete".into(),
        expected_durable_count: 0,
        force: false,
    }
}
fn setup() -> (
    Arc<AgentSessionService>,
    Arc<SqliteConversationCatalog>,
    tempfile::TempDir,
) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now');
        insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');
        insert into agent_conversations(id,title,archived) values('00000000-0000-0000-0000-000000000001','Delete',1);
        insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values('00000000-0000-0000-0000-000000000001','manuscript','m');").unwrap();
    let root = tempfile::tempdir().unwrap();
    let db = crate::data_interface::Database::open(db, &root.path().join("log")).unwrap();
    let store = Arc::new(SqliteConversationStore::new(db.clone(), "Delete".into()).unwrap());
    (
        Arc::new(AgentSessionService::new(store)),
        Arc::new(SqliteConversationCatalog::new(db)),
        root,
    )
}

#[tokio::test]
async fn interrupted_batch_does_not_publish_after_waiting_for_session_owner() {
    use research_agent_core::conversation::ConversationCatalog;
    let (service, catalog, _root) = setup();
    let request = request();
    let slot = service.slot(request.conversation_id, true).unwrap();
    let held = slot.lock().await;
    let worker = service.clone();
    let batch_catalog = catalog.as_ref().clone();
    let batch_request = request.clone();
    let task = tokio::spawn(async move {
        worker
            .delete_conversations_batch(batch_catalog, vec![batch_request])
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if service
                .deleting
                .lock()
                .unwrap()
                .contains(&request.conversation_id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    drop(held);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if !service
                .deleting
                .lock()
                .unwrap()
                .contains(&request.conversation_id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        catalog
            .read(request.conversation_id, request.binding.clone())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn shutdown_stops_an_unsubmitted_batch_instead_of_continuing_it() {
    use research_agent_core::conversation::ConversationCatalog;
    let (service, catalog, _root) = setup();
    let request = request();
    let slot = service.slot(request.conversation_id, true).unwrap();
    let held = slot.lock().await;
    let worker = service.clone();
    let batch_catalog = catalog.as_ref().clone();
    let batch_request = request.clone();
    let batch = tokio::spawn(async move {
        worker
            .delete_conversations_batch(batch_catalog, vec![batch_request])
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !service
            .deleting
            .lock()
            .unwrap()
            .contains(&request.conversation_id)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let stopping = service.clone();
    let shutdown = tokio::spawn(async move { stopping.tasks.shutdown().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !service.tasks.is_closing().unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(held);
    assert!(batch.await.unwrap().is_err());
    shutdown.await.unwrap().unwrap();
    assert!(
        catalog
            .read(request.conversation_id, request.binding.clone())
            .await
            .is_ok()
    );
}

#[test]
fn batch_fence_slots_do_not_consume_the_open_session_limit() {
    let (service, _catalog, _root) = setup();
    let mut held = Vec::new();
    for n in 100..100 + MAX_OPEN_SESSIONS + 10 {
        let id = ConversationId::from_uuid(Uuid::from_u128(n as u128));
        let slot = Arc::new(AsyncMutex::new(None));
        service.sessions.lock().unwrap().insert(id, slot.clone());
        service.batch_fence_slots.lock().unwrap().insert(id);
        held.push(slot);
    }
    assert!(
        service
            .slot(ConversationId::from_uuid(Uuid::from_u128(9000)), true)
            .is_ok()
    );
    drop(held);
}
struct PausedStore {
    catalog: Arc<SqliteConversationCatalog>,
    entered: Notify,
    release: Notify,
    done: Notify,
}

#[tokio::test]
async fn shutdown_drains_in_flight_deletion_before_releasing_sessions() {
    let (service, catalog, _root) = setup();
    let store = Arc::new(PausedStore {
        catalog,
        entered: Notify::new(),
        release: Notify::new(),
        done: Notify::new(),
    });
    let worker = service.clone();
    let paused = store.clone();
    let deletion = tokio::spawn(async move { worker.delete_conversation(paused, request()).await });
    store.entered.notified().await;
    let shutdown = service.shutdown();
    tokio::pin!(shutdown);
    tokio::select! { biased; result = &mut shutdown => panic!("shutdown escaped deletion: {result:?}"), _ = std::future::ready(()) => {} }
    assert!(matches!(
        service
            .tasks
            .reserve_idle(ConversationId::from_uuid(Uuid::from_u128(90))),
        Err(TaskRegistryError::Closing)
    ));
    store.release.notify_one();
    deletion.await.unwrap().unwrap();
    shutdown.await.unwrap();
}
impl ConversationDeletionStore for PausedStore {
    fn delete(&self, request: ConversationDeletionRequest) -> ConversationDeletionFuture<'_> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            let result = self.catalog.delete(request).await;
            self.done.notify_one();
            result
        })
    }
}

#[tokio::test]
async fn deleted_idle_owner_is_not_returned_and_exact_retry_survives() {
    let (service, catalog, _root) = setup();
    let request = request();
    service.open(request.conversation_id).await.unwrap();
    service.observe_history_snapshot(request.conversation_id, 0);
    assert!(
        !service
            .wait_for_accepted_history(request.conversation_id, 0, 0)
            .await
            .unwrap()
            .resync_required
    );
    let first = service
        .delete_conversation(catalog.clone(), request.clone())
        .await
        .unwrap();
    assert!(
        service
            .wait_for_accepted_history(request.conversation_id, 0, 0)
            .await
            .unwrap()
            .resync_required
    );
    assert!(service.open(request.conversation_id).await.is_err());
    assert!(service.create(request.conversation_id).await.is_err());
    assert_eq!(
        service.delete_conversation(catalog, request).await.unwrap(),
        first
    );
}

#[tokio::test]
async fn dropping_waiter_keeps_reservation_and_blocks_queued_opens_until_delete_finishes() {
    let (service, catalog, _root) = setup();
    let request = request();
    let duplicate_request = request.clone();
    let id = request.conversation_id;
    service.open(id).await.unwrap();
    let store = Arc::new(PausedStore {
        catalog,
        entered: Notify::new(),
        release: Notify::new(),
        done: Notify::new(),
    });
    let worker = service.clone();
    let paused = store.clone();
    let caller = tokio::spawn(async move { worker.delete_conversation(paused, request).await });
    store.entered.notified().await;
    assert!(matches!(
        service
            .delete_conversation(store.clone(), duplicate_request)
            .await,
        Err(Error::Catalog(CatalogError::Busy))
    ));
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(matches!(
        service.start_owned_turn(id, |_, _| async { Ok(()) }),
        Err(TaskRegistryError::Active)
    ));
    assert!(matches!(
        service.tasks.reserve_idle(id),
        Err(TaskRegistryError::Active)
    ));
    // Poll the open into the occupied slot; no timing-dependent sleep.
    let open = service.open(id);
    tokio::pin!(open);
    tokio::select! { biased; result = &mut open => panic!("open escaped deletion lock: {result:?}"), _ = std::future::ready(()) => {} }
    store.release.notify_one();
    assert!(open.await.is_err());
    store.done.notified().await;
    assert!(service.tasks.reserve_idle(id).is_ok());
}

#[tokio::test]
async fn active_task_before_first_event_and_active_history_both_refuse_deletion() {
    let (service, catalog, _root) = setup();
    let id = request().conversation_id;
    service.open(id).await.unwrap();
    let (send, wait) = tokio::sync::oneshot::channel();
    let owned = service
        .start_owned_turn(id, |_, _| async move {
            wait.await.unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        service
            .delete_conversation(catalog.clone(), request())
            .await
            .unwrap_err(),
        Error::Catalog(CatalogError::Busy)
    );
    send.send(()).unwrap();
    owned.task.wait().await;
    use research_agent_core::conversation::{ArchiveConversation, ConversationCatalog};
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: false,
            expected_watermark: None,
        })
        .await
        .unwrap();
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: Uuid::new_v4(),
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .delete_conversation(catalog, request())
            .await
            .unwrap_err(),
        Error::Catalog(CatalogError::Busy)
    );
    assert!(service.open(id).await.unwrap().active_turn().is_some());
}

#[tokio::test]
async fn invalid_input_does_not_release_owner_and_failed_confirmation_allows_reopen() {
    let (service, catalog, _root) = setup();
    let id = request().conversation_id;
    service.open(id).await.unwrap();
    let slot = service.slot(id, false).unwrap();
    let mut invalid = request();
    invalid.request_id = Uuid::nil();
    assert_eq!(
        service
            .delete_conversation(catalog.clone(), invalid)
            .await
            .unwrap_err(),
        Error::Catalog(CatalogError::InvalidInput)
    );
    assert!(slot.lock().await.is_some());
    let mut stale = request();
    stale.expected_title = "Changed".into();
    assert_eq!(
        service
            .delete_conversation(catalog, stale)
            .await
            .unwrap_err(),
        Error::Catalog(CatalogError::Conflict)
    );
    assert!(slot.lock().await.is_none());
    assert!(service.tasks.reserve_idle(id).is_ok());
    service.open(id).await.unwrap();
}
