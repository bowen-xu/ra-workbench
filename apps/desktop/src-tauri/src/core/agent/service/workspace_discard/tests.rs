use super::*;
use crate::agent::repository::execution_workspaces::Binding;
use crate::storage::AppStore;
use research_agent_core::protocol::{EventPayload, ModelIdentity};
use rusqlite::Connection;
use tokio::sync::Notify;

fn setup() -> (
    tempfile::TempDir,
    AppStore,
    Arc<AgentSessionService>,
    Request,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 89);
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut db,
        ConversationId::from_uuid(id),
        "fixture",
    )
    .unwrap();
    let store = AppStore::for_test(db, temp.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let request = Request {
        operation_id: Uuid::new_v4(),
        binding: Binding {
            conversation_id: id,
            request_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            manuscript_id: Uuid::new_v4(),
            lease_id: Uuid::new_v4(),
            manifest_hash: "a".repeat(64),
        },
        environment_id: "fixture".into(),
        inventory_hash: "b".repeat(64),
    };
    (temp, store, service, request)
}

#[tokio::test]
async fn cancelled_waiter_keeps_admission_and_session_until_worker_settles() {
    let (_temp, _store, service, request) = setup();
    let id = ConversationId::from_uuid(request.binding.conversation_id);
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let done = Arc::new(Notify::new());
    let worker = service.clone();
    let (start, gate, end) = (entered.clone(), release.clone(), done.clone());
    let caller = tokio::spawn(async move {
        worker
            .discard_workspace_with(request, move || async move {
                start.notify_one();
                gate.notified().await;
                end.notify_one();
                Ok(())
            })
            .await
    });
    entered.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(matches!(
        service.tasks.reserve_idle(id),
        Err(TaskRegistryError::Active)
    ));
    let slot = service.slot(id, false).unwrap();
    assert!(slot.try_lock().is_err());
    let shutdown = service.shutdown();
    tokio::pin!(shutdown);
    tokio::select! { biased; result = &mut shutdown => panic!("shutdown escaped discard: {result:?}"), _ = std::future::ready(()) => {} }
    assert!(matches!(
        service
            .tasks
            .reserve_idle(ConversationId::from_uuid(Uuid::new_v4())),
        Err(TaskRegistryError::Closing)
    ));
    release.notify_one();
    done.notified().await;
    shutdown.await.unwrap();
}

#[tokio::test]
async fn active_registry_before_first_event_refuses_discard_without_calling_storage() {
    let (_temp, _store, service, request) = setup();
    let id = ConversationId::from_uuid(request.binding.conversation_id);
    let release = Arc::new(Notify::new());
    let gate = release.clone();
    let running = service
        .tasks
        .start(id, move |_| async move {
            gate.notified().await;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        service
            .discard_workspace_with(request, || async { panic!("storage must not run") })
            .await,
        Err(Error::Lease(LeaseError::Busy))
    ));
    release.notify_one();
    running.wait().await;
}

#[tokio::test]
async fn persisted_active_turn_is_not_implicitly_interrupted_for_cleanup() {
    let (_temp, store, service, request) = setup();
    let id = ConversationId::from_uuid(request.binding.conversation_id);
    service.open(id).await.unwrap();
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: request.binding.turn_id,
                context_snapshot_id: request.binding.context_snapshot_id,
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
        )
        .await
        .unwrap();
    drop(service);
    let service = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    assert!(matches!(
        service
            .discard_workspace_with(request, || async { panic!("storage must not run") })
            .await,
        Err(Error::Lease(LeaseError::Busy))
    ));
    let slot = service.slot(id, false).unwrap();
    assert!(
        slot.lock()
            .await
            .as_ref()
            .unwrap()
            .projection()
            .active_turn()
            .is_some()
    );
}

#[tokio::test]
async fn failed_worker_releases_gate_and_keeps_idle_history_available() {
    let (_temp, _store, service, request) = setup();
    let id = ConversationId::from_uuid(request.binding.conversation_id);
    assert!(matches!(
        service
            .discard_workspace_with(request, || async { Err(Error::RecoveryRequired) })
            .await,
        Err(Error::RecoveryRequired)
    ));
    assert!(service.tasks.reserve_idle(id).is_ok());
    assert!(service.open(id).await.unwrap().active_turn().is_none());
}
