use super::*;
use research_agent_core::protocol::{EventPayload, ModelIdentity, TurnPhase};

async fn begin(service: &AgentSessionService, id: ConversationId, turn_id: Uuid) {
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id,
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "test".into(),
                    model_id: "test".into(),
                    revision: None,
                },
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn activity_invalidation_skips_text_and_failed_commits() {
    use research_agent_core::protocol::ItemKind;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (connection, _, mut service, id) = super::super::tests::setup();
    let changed = Arc::new(AtomicUsize::new(0));
    let observer = changed.clone();
    service.activity_changed = Some(Arc::new(move || {
        observer.fetch_add(1, Ordering::SeqCst);
    }));
    service.create(id).await.unwrap();
    let turn = Uuid::new_v4();
    begin(&service, id, turn).await;
    assert_eq!(changed.load(Ordering::SeqCst), 1);
    let item = Uuid::new_v4();
    service
        .commit_payloads(
            id,
            Uuid::new_v4(),
            vec![
                EventPayload::PhaseChanged {
                    turn_id: turn,
                    phase: TurnPhase::RunningModel,
                },
                EventPayload::ItemStarted {
                    turn_id: turn,
                    item_id: item,
                    kind: ItemKind::AssistantMessage,
                },
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text: "accepted".into(),
                },
            ],
        )
        .await
        .unwrap();
    assert_eq!(changed.load(Ordering::SeqCst), 1);
    connection.lock().unwrap().execute_batch("create trigger fail_activity_event before insert on agent_items begin select raise(abort,'synthetic'); end;").unwrap();
    assert!(
        service
            .commit_payload(
                id,
                Uuid::new_v4(),
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text: "failed".into()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(changed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn wait_sees_existing_and_racing_commits_and_zero_wait_is_an_immediate_check() {
    let (_, _, service, id) = super::super::tests::setup();
    service.create(id).await.unwrap();
    assert_eq!(service.wait_for_history(id, 0, 0).await.unwrap(), None);
    let turn = Uuid::new_v4();
    begin(&service, id, turn).await;
    assert_eq!(service.wait_for_history(id, 0, 0).await.unwrap(), Some(1));
    let mut wait = Box::pin(service.wait_for_history(id, 1, 30_000));
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(wait.as_mut().poll(cx).is_pending()))
            .await
    );
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::PhaseChanged {
                turn_id: turn,
                phase: TurnPhase::RunningModel,
            },
        )
        .await
        .unwrap();
    assert_eq!(wait.await.unwrap(), Some(2));
    assert!(service.wait_for_history(id, 3, 0).await.is_err());
}

#[tokio::test]
async fn canceled_wait_releases_subscription_and_admission_is_bounded() {
    let (_, _, service, id) = super::super::tests::setup();
    service.create(id).await.unwrap();
    let mut wait = Box::pin(service.wait_for_history(id, 0, 30_000));
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(wait.as_mut().poll(cx).is_pending()))
            .await
    );
    assert_eq!(service.update_waiters.available_permits(), 127);
    assert_eq!(service.notices.receiver_count(), 1);
    drop(wait);
    assert_eq!(service.update_waiters.available_permits(), 128);
    assert_eq!(service.notices.receiver_count(), 0);
    let _capacity = service.update_waiters.acquire_many(128).await.unwrap();
    assert!(matches!(
        service.wait_for_history(id, 0, 0).await,
        Err(AgentServiceError::Capacity)
    ));
}

#[tokio::test]
async fn native_wait_delivers_accepted_events_and_resync_without_sql_reads() {
    use std::{future::Future, task::Context};
    let (db, _, service, id) = super::super::tests::setup();
    service.create(id).await.unwrap();
    let turn = Uuid::new_v4();
    begin(&service, id, turn).await;
    // A SQL-reading implementation would suspend on this held connection.
    let _guard = db.lock().unwrap();
    let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
    let mut wait = Box::pin(service.wait_for_accepted_history(id, 0, 0));
    let update = match wait.as_mut().poll(&mut context) {
        std::task::Poll::Ready(Ok(update)) => update,
        _ => panic!("native accepted wait must not read SQLite"),
    };
    assert_eq!(update.watermark, Some(1));
    assert_eq!(update.events.len(), 1);
    assert_eq!(update.events[0].seq(), 1);
    assert!(!update.resync_required);
    service.invalidate_accepted_history(id);
    let mut missing = Box::pin(service.wait_for_accepted_history(id, 1, 0));
    assert!(
        matches!(missing.as_mut().poll(&mut context),std::task::Poll::Ready(Ok(update)) if update.resync_required && update.events.is_empty())
    );
    service.observe_history_snapshot(id, 1);
    let mut idle = Box::pin(service.wait_for_accepted_history(id, 1, 0));
    assert!(
        matches!(idle.as_mut().poll(&mut context),std::task::Poll::Ready(Ok(update)) if !update.resync_required && update.watermark.is_none())
    );
}
