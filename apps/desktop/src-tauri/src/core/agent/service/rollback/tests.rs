use super::*;
use crate::agent::repository::{SqliteConversationStore, context_windows};
use research_agent_core::{
    context::{checkpoint::ContextCheckpoint, compaction_strategy::*},
    protocol::{ItemKind, ModelIdentity, TaskKind, TurnOutcome},
};
use rusqlite::Connection;

pub(super) fn setup() -> (
    Arc<Mutex<Connection>>,
    Arc<AgentSessionService>,
    ConversationId,
) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let db = Arc::new(Mutex::new(db));
    let store = Arc::new(SqliteConversationStore::new(db.clone(), "Rollback".into()).unwrap());
    let service = Arc::new(AgentSessionService::new_with_compaction_db(
        store,
        Some(db.clone()),
    ));
    (db, service, ConversationId::from_uuid(Uuid::new_v4()))
}
pub(super) fn model() -> ModelIdentity {
    ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: None,
    }
}
pub(super) async fn start(service: &AgentSessionService, id: ConversationId) -> Uuid {
    let turn_id = Uuid::new_v4();
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: TaskKind::Regular,
                turn_id,
                context_snapshot_id: Uuid::new_v4(),
                model: model(),
            },
        )
        .await
        .unwrap();
    turn_id
}
pub(super) async fn finish(service: &AgentSessionService, id: ConversationId, turn_id: Uuid) {
    let item_id = Uuid::new_v4();
    service
        .commit_payloads(
            id,
            Uuid::new_v4(),
            vec![
                EventPayload::ItemStarted {
                    turn_id,
                    item_id,
                    kind: ItemKind::UserMessage,
                },
                EventPayload::TextDelta {
                    turn_id,
                    item_id,
                    text: "user evidence".into(),
                },
                EventPayload::ItemCompleted { turn_id, item_id },
                EventPayload::TurnFinished {
                    turn_id,
                    outcome: TurnOutcome::Completed,
                },
            ],
        )
        .await
        .unwrap();
}
fn request(id: ConversationId, turn_id: Uuid, watermark: u64) -> ConversationRollbackRequest {
    ConversationRollbackRequest {
        conversation_id: id,
        request_id: Uuid::new_v4(),
        expected_watermark: watermark,
        expected_last_turn: turn_id,
        user_turns: 1,
    }
}

#[tokio::test]
async fn rollback_updates_live_owner_and_window_replay_does_not_remove_later_turn() {
    let (db, service, id) = setup();
    service.create(id).await.unwrap();
    let first = start(&service, id).await;
    finish(&service, id, first).await;
    let second = start(&service, id).await;
    let window = ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::new_v4(),
        conversation_id: *id.as_uuid(),
        turn_id: second,
        source_watermark: 6,
        previous_window_id: None,
        binding: CompactionBinding {
            model: model(),
            strategy_id: "fixture.text".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::TextSummary("state to undo".into()),
        retained: vec![],
        canonical_context: None,
    };
    context_windows::install(&mut db.lock().unwrap(), &window).unwrap();
    finish(&service, id, second).await;
    let operation = request(id, second, 10);
    let mut notices = service.subscribe();
    let receipt = service.rollback_conversation(operation).await.unwrap();
    assert_eq!(receipt.watermark, 11);
    assert_eq!(notices.try_recv().unwrap().watermark, 11);
    assert!(
        context_windows::read_active(&db.lock().unwrap(), *id.as_uuid())
            .unwrap()
            .is_none()
    );
    let live = service.open(id).await.unwrap();
    assert_eq!(live.turn_order(), &[first]);
    let third = start(&service, id).await;
    finish(&service, id, third).await;
    while notices.try_recv().is_ok() {}
    assert_eq!(
        service
            .rollback_conversation(operation)
            .await
            .unwrap()
            .events,
        receipt.events
    );
    assert!(notices.try_recv().is_err());
    assert_eq!(
        service.open(id).await.unwrap().turn_order(),
        &[first, third]
    );
    service.release_idle(id).await.unwrap();
    assert_eq!(
        service.open(id).await.unwrap().turn_order(),
        &[first, third]
    );
    assert_eq!(service.read_since(id, 0, 30).await.unwrap().items.len(), 16);
    let mut changed = operation;
    changed.user_turns = 2;
    assert!(matches!(
        service.rollback_conversation(changed).await,
        Err(AgentServiceError::RollbackConflict)
    ));
}

#[tokio::test]
async fn active_stale_and_failed_writes_publish_no_rollback() {
    let (db, service, id) = setup();
    service.create(id).await.unwrap();
    let turn = start(&service, id).await;
    assert!(matches!(
        service.rollback_conversation(request(id, turn, 1)).await,
        Err(AgentServiceError::Active)
    ));
    finish(&service, id, turn).await;
    assert!(matches!(
        service.rollback_conversation(request(id, turn, 4)).await,
        Err(AgentServiceError::RollbackConflict)
    ));
    let reservation = service.tasks.reserve_idle(id).unwrap();
    assert!(matches!(
        service.rollback_conversation(request(id, turn, 5)).await,
        Err(AgentServiceError::Active)
    ));
    drop(reservation);
    db.lock().unwrap().execute_batch("create trigger fault before insert on agent_journal_records when new.ordinal=5 begin select raise(abort,'synthetic rollback failure'); end;").unwrap();
    let mut notices = service.subscribe();
    assert!(
        service
            .rollback_conversation(request(id, turn, 5))
            .await
            .is_err()
    );
    assert!(notices.try_recv().is_err());
    assert!(matches!(
        service.open(id).await,
        Err(AgentServiceError::Session(SessionError::RecoveryRequired))
    ));
    assert_eq!(
        service.read_since(id, 0, 20).await.unwrap().durable_count,
        5
    );
    assert_eq!(service.recover(id).await.unwrap().turn_order(), &[turn]);
}

#[tokio::test]
async fn dropping_waiter_keeps_admission_until_owned_rollback_commits() {
    let (_, service, id) = setup();
    service.create(id).await.unwrap();
    let turn = start(&service, id).await;
    finish(&service, id, turn).await;
    let slot = service.slot(id, false).unwrap();
    let owner = slot.lock().await;
    let mut notices = service.subscribe();
    let worker = service.clone();
    let caller =
        tokio::spawn(async move { worker.rollback_conversation(request(id, turn, 5)).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match service.tasks.reserve_idle(id) {
                Err(TaskRegistryError::Active) => break,
                Ok(reservation) => drop(reservation),
                Err(error) => panic!("unexpected admission error: {error}"),
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(matches!(
        service.tasks.reserve_idle(id),
        Err(TaskRegistryError::Active)
    ));
    drop(owner);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), notices.recv())
            .await
            .unwrap()
            .unwrap()
            .watermark,
        6
    );
    assert!(service.open(id).await.unwrap().turn_order().is_empty());
    assert!(service.tasks.reserve_idle(id).is_ok());
}
