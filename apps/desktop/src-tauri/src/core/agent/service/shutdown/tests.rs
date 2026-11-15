use super::*;
use crate::agent::tasks::{TaskCompletion, TaskRegistryError};
use research_agent_core::{
    model::ModelError,
    protocol::{EventPayload, ModelIdentity, TurnOutcome},
};
use tokio::sync::oneshot;

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
async fn shutdown_waits_for_task_and_durable_terminal_before_releasing_writer() {
    let (_, _, service, id) = super::super::tests::setup();
    let service = Arc::new(service);
    service.create(id).await.unwrap();
    let worker = service.clone();
    let (entered, ready) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let owned = service
        .start_owned_turn(id, move |turn, signal| async move {
            begin(&worker, id, turn).await;
            entered.send(()).unwrap();
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
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    ready.await.unwrap();
    let mut shutdown = Box::pin(service.shutdown());
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(shutdown.as_mut().poll(cx).is_pending()))
            .await
    );
    assert!(matches!(
        service.start_owned_turn(id, |_, _| async { Ok(()) }),
        Err(TaskRegistryError::Closing)
    ));
    release.send(()).unwrap();
    shutdown.await.unwrap();
    assert_eq!(owned.task.wait().await, TaskCompletion::Canceled);
    let page = service.read_since(id, 0, 10).await.unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(matches!(
        page.items[1].payload(),
        EventPayload::TurnFinished {
            outcome: TurnOutcome::Canceled,
            ..
        }
    ));
    service.shutdown().await.unwrap();
    assert_eq!(service.read_since(id, 0, 10).await.unwrap().items.len(), 2);
}

#[tokio::test]
async fn failed_shutdown_can_retry_without_losing_or_replaying_events() {
    let (connection, _, service, id) = super::super::tests::setup();
    service.create(id).await.unwrap();
    begin(&service, id, Uuid::new_v4()).await;
    connection.lock().unwrap().execute_batch("create trigger fail_shutdown before insert on agent_items begin select raise(abort, 'injected'); end;").unwrap();
    assert!(service.shutdown().await.is_err());
    assert_eq!(service.read_since(id, 0, 10).await.unwrap().items.len(), 1);
    connection
        .lock()
        .unwrap()
        .execute_batch("drop trigger fail_shutdown;")
        .unwrap();
    service.shutdown().await.unwrap();
    let page = service.read_since(id, 0, 10).await.unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(matches!(
        page.items[1].payload(),
        EventPayload::TurnFinished {
            outcome: TurnOutcome::Interrupted,
            ..
        }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn application_shutdown_drains_processes_and_closes_execution_admission() {
    use crate::agent::process::{ProcessLimits, ProcessRegistryError, ProcessSession, ProcessSpec};
    use research_agent_core::workspace::WorkspaceIdentity;
    use std::{collections::BTreeMap, time::Duration};
    let (_, _, service, id) = super::super::tests::setup();
    let temp = tempfile::tempdir().unwrap();
    service.create(id).await.unwrap();
    let scope = WorkspaceIdentity {
        lease_id: Uuid::new_v4(),
        conversation_id: *id.as_uuid(),
        turn_id: Uuid::new_v4(),
        environment_id: "test".into(),
    };
    service
        .processes()
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let process = service
        .processes()
        .start(scope.clone(), || {
            ProcessSession::spawn(
                ProcessSpec {
                    program: "/bin/cat".into(),
                    arguments: vec![],
                    directory: temp.path().into(),
                    environment: BTreeMap::new(),
                },
                ProcessLimits {
                    retained_bytes: 128,
                    execution_timeout: Duration::from_secs(10),
                    drain_timeout: Duration::from_secs(2),
                },
            )
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), service.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        service
            .processes()
            .poll(&scope, process, Duration::ZERO)
            .await,
        Err(ProcessRegistryError::NotFound)
    ));
    assert!(matches!(
        service
            .processes()
            .start(scope, || panic!("closed application must not launch")),
        Err(ProcessRegistryError::Closing)
    ));
}
