use super::*;
use research_agent_core::protocol::{EventPayload, ModelIdentity, TurnOutcome};

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
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn panic_is_reconciled_before_task_receipt_completes() {
    let (_, _, service, id) = super::super::tests::setup();
    let service = Arc::new(service);
    service.create(id).await.unwrap();
    let worker = service.clone();
    let owned = service
        .start_owned_turn(id, move |turn, _| async move {
            begin(&worker, id, turn).await;
            panic!("synthetic failure after durable turn start");
        })
        .unwrap();
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    let projection = service.open(id).await.unwrap();
    assert_eq!(projection.active_turn(), None);
    assert_eq!(
        projection.turn(owned.turn_id).unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(projection.seq(), 2);
    assert!(
        !service
            .reconcile_stopped_turn(id, owned.turn_id)
            .await
            .unwrap()
    );
    assert_eq!(service.open(id).await.unwrap().seq(), 2);
}

#[tokio::test]
async fn completed_terminal_is_not_rewritten_and_late_reconciliation_is_fenced() {
    let (_, _, service, id) = super::super::tests::setup();
    let service = Arc::new(service);
    service.create(id).await.unwrap();
    let worker = service.clone();
    let owned = service
        .start_owned_turn(id, move |turn, _| async move {
            begin(&worker, id, turn).await;
            worker
                .commit_payload(
                    id,
                    Uuid::new_v4(),
                    EventPayload::TurnFinished {
                        turn_id: turn,
                        outcome: TurnOutcome::Completed,
                    },
                )
                .await
                .unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert_eq!(service.open(id).await.unwrap().seq(), 2);
    let next = Uuid::new_v4();
    begin(&service, id, next).await;
    assert!(matches!(
        service.reconcile_stopped_turn(id, owned.turn_id).await,
        Err(AgentServiceError::Active)
    ));
    assert_eq!(service.open(id).await.unwrap().active_turn(), Some(next));
}

#[tokio::test]
async fn success_without_terminal_is_reported_as_recovery_instead_of_completed() {
    let (_, _, service, id) = super::super::tests::setup();
    let service = Arc::new(service);
    service.create(id).await.unwrap();
    let worker = service.clone();
    let owned = service
        .start_owned_turn(id, move |turn, _| async move {
            begin(&worker, id, turn).await;
            Ok(())
        })
        .unwrap();
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(service.open(id).await.unwrap().active_turn(), None);
}

#[tokio::test]
async fn uncertain_writer_is_reopened_before_reconciling_the_stopped_turn() {
    let (connection, _, service, id) = super::super::tests::setup();
    let service = Arc::new(service);
    service.create(id).await.unwrap();
    let worker = service.clone();
    let owned = service.start_owned_turn(id, move |turn, _| async move {
        begin(&worker, id, turn).await;
        connection.lock().unwrap().execute_batch("create trigger fail_owned before insert on agent_items begin select raise(abort, 'injected'); end;").unwrap();
        let result = worker.commit_payload(id, Uuid::new_v4(), EventPayload::TurnFinished {
            turn_id: turn, outcome: TurnOutcome::Completed,
        }).await;
        assert!(result.is_err());
        connection.lock().unwrap().execute_batch("drop trigger fail_owned;").unwrap();
        Err(RunError::EventSink)
    }).unwrap();
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    let projection = service.open(id).await.unwrap();
    assert_eq!(projection.seq(), 2);
    assert_eq!(
        projection.turn(owned.turn_id).unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
}

#[tokio::test]
async fn durable_turn_identity_can_cancel_after_receipt_is_dropped() {
    let (_, _, service, id) = super::super::tests::setup();
    let service = Arc::new(service);
    service.create(id).await.unwrap();
    let worker = service.clone();
    let (ready, started) = tokio::sync::oneshot::channel();
    let (release, gate) = tokio::sync::oneshot::channel();
    let owned = service
        .start_owned_turn(id, move |turn, signal| async move {
            begin(&worker, id, turn).await;
            ready.send(()).unwrap();
            gate.await.unwrap();
            assert!(signal.is_cancelled());
            worker
                .commit_payload(
                    id,
                    Uuid::new_v4(),
                    EventPayload::TurnFinished {
                        turn_id: turn,
                        outcome: TurnOutcome::Canceled,
                    },
                )
                .await
                .unwrap();
            Err(research_agent_core::model::ModelError::Canceled.into())
        })
        .unwrap();
    let turn_id = owned.turn_id;
    drop(owned);
    started.await.unwrap();
    let restored = service.open(id).await.unwrap();
    assert_eq!(restored.active_turn(), Some(turn_id));
    assert!(service.tasks().cancel_turn(id, turn_id).unwrap());
    assert_eq!(service.open(id).await.unwrap().active_turn(), Some(turn_id));
    release.send(()).unwrap();
    service.tasks().shutdown().await.unwrap();
    assert_eq!(
        service
            .open(id)
            .await
            .unwrap()
            .turn(turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Canceled)
    );
    assert!(!service.tasks().cancel_turn(id, turn_id).unwrap());
}
