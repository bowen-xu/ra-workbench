use super::SqliteConversationStore;
use research_agent_core::{
    conversation::{ConversationId, WriterId, WriterLease},
    protocol::{AgentEvent, EventPayload, ModelIdentity, TurnOutcome, TurnPhase},
    session::{DurableConversation, SessionError},
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
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
    )
}
fn setup() -> (
    Arc<Mutex<Connection>>,
    Arc<SqliteConversationStore<AgentEvent>>,
    ConversationId,
) {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 67);
    let connection = Arc::new(Mutex::new(connection));
    let store =
        Arc::new(SqliteConversationStore::new(connection.clone(), "Session".into()).unwrap());
    (connection, store, ConversationId::from_uuid(Uuid::new_v4()))
}

#[tokio::test]
async fn invalid_batch_is_rejected_before_storage_or_projection_changes() {
    let (connection, store, id) = setup();
    let mut session = DurableConversation::create(store, lease(id)).await.unwrap();
    let invalid = event(
        id,
        3,
        EventPayload::TurnFinished {
            turn_id: Uuid::from_u128(2),
            outcome: TurnOutcome::Completed,
        },
    );
    assert!(session.commit(vec![start(id), invalid]).await.is_err());
    assert_eq!(session.projection().seq(), 0);
    assert!(!session.needs_recovery());
    let count: i64 = connection
        .lock()
        .unwrap()
        .query_row("select count(*) from agent_items", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(session.commit(vec![start(id)]).await.unwrap().watermark, 1);
}

#[tokio::test]
async fn failed_flush_publishes_nothing_and_requires_reopening_before_new_writes() {
    let (connection, store, id) = setup();
    let mut session = DurableConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    connection.lock().unwrap().execute_batch("create trigger fail_event before insert on agent_items begin select raise(abort, 'injected'); end;").unwrap();
    assert!(matches!(
        session.commit(vec![start(id)]).await,
        Err(SessionError::Storage(_))
    ));
    assert_eq!(session.projection().seq(), 0);
    assert!(session.needs_recovery());
    assert!(matches!(
        session.commit(vec![start(id)]).await,
        Err(SessionError::RecoveryRequired)
    ));
    drop(session);
    connection
        .lock()
        .unwrap()
        .execute_batch("drop trigger fail_event;")
        .unwrap();
    let mut restored = DurableConversation::resume(store, lease(id)).await.unwrap();
    assert_eq!(restored.projection().seq(), 0);
    let receipt = restored.commit(vec![start(id)]).await.unwrap();
    assert_eq!(receipt.events.len(), 1);
    assert_eq!(restored.projection().seq(), receipt.watermark);
}

#[tokio::test]
async fn canceling_commit_keeps_recovery_barrier_even_if_owner_remains_alive() {
    let (connection, store, id) = setup();
    let mut session = DurableConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    {
        let guard = connection.lock().unwrap();
        let mut pending = Box::pin(session.commit(vec![start(id)]));
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(pending);
        drop(guard);
    }
    assert!(session.needs_recovery());
    assert_eq!(session.projection().seq(), 0);
    assert!(matches!(
        session.commit(vec![start(id)]).await,
        Err(SessionError::RecoveryRequired)
    ));
    drop(session);
    let recovered = DurableConversation::resume(store, lease(id)).await.unwrap();
    assert_eq!(recovered.projection().seq(), 0);
}

#[tokio::test]
async fn recovery_marks_active_turn_interrupted_once_without_reexecuting_it() {
    let (_, store, id) = setup();
    let mut session = DurableConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    session.commit(vec![start(id)]).await.unwrap();
    drop(session);
    let mut recovered = DurableConversation::resume(store.clone(), lease(id))
        .await
        .unwrap();
    assert_eq!(
        recovered.projection().active_turn(),
        Some(Uuid::from_u128(2))
    );
    let receipt = recovered
        .interrupt_recovered_turn(Uuid::new_v4(), 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.watermark, 2);
    assert!(
        recovered
            .interrupt_recovered_turn(Uuid::new_v4(), 2)
            .await
            .unwrap()
            .is_none()
    );
    drop(recovered);
    let restored = DurableConversation::resume(store, lease(id)).await.unwrap();
    assert_eq!(
        restored
            .projection()
            .turn(Uuid::from_u128(2))
            .unwrap()
            .outcome,
        Some(TurnOutcome::Interrupted)
    );
}

#[tokio::test]
async fn resume_replays_all_pages_before_allowing_new_commits() {
    let (_, store, id) = setup();
    let mut session = DurableConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    session.commit(vec![start(id)]).await.unwrap();
    for range in [2..302, 302..602] {
        let batch = range
            .map(|seq| {
                event(
                    id,
                    seq,
                    EventPayload::PhaseChanged {
                        turn_id: Uuid::from_u128(2),
                        phase: TurnPhase::RunningModel,
                    },
                )
            })
            .collect();
        session.commit(batch).await.unwrap();
    }
    let expected = session.projection().clone();
    drop(session);
    let mut recovered = DurableConversation::resume(store, lease(id)).await.unwrap();
    assert_eq!(recovered.projection(), &expected);
    assert_eq!(
        recovered
            .interrupt_recovered_turn(Uuid::new_v4(), 0)
            .await
            .unwrap()
            .unwrap()
            .watermark,
        602
    );
}

#[tokio::test]
async fn recovery_preserves_a_terminal_diagnostic_if_the_process_stopped_before_turn_finished() {
    let (_, store, id) = setup();
    let mut session = DurableConversation::create(store.clone(), lease(id))
        .await
        .unwrap();
    session
        .commit(vec![
            start(id),
            event(
                id,
                2,
                EventPayload::TurnFailed {
                    turn_id: Uuid::from_u128(2),
                    error: "run budget exhausted".into(),
                    category: research_agent_core::protocol::FailureCategory::Admission,
                },
            ),
        ])
        .await
        .unwrap();
    drop(session);
    let mut recovered = DurableConversation::resume(store.clone(), lease(id))
        .await
        .unwrap();
    recovered
        .interrupt_recovered_turn(Uuid::new_v4(), 1)
        .await
        .unwrap()
        .unwrap();
    assert!(
        recovered
            .interrupt_recovered_turn(Uuid::new_v4(), 2)
            .await
            .unwrap()
            .is_none()
    );
    drop(recovered);
    let restored = DurableConversation::resume(store, lease(id)).await.unwrap();
    let turn = restored.projection().turn(Uuid::from_u128(2)).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
    assert_eq!(
        turn.failure,
        Some((
            "run budget exhausted".into(),
            research_agent_core::protocol::FailureCategory::Admission
        ))
    );
}
