use super::*;
use crate::agent::process::{
    ProcessLimits, ProcessPoll, ProcessRegistryError, ProcessSession, ProcessSpec, StopReason,
};
use research_agent_core::workspace::WorkspaceIdentity;
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::{sync::oneshot, time::timeout};
fn launch(path: &Path) -> Result<ProcessSession, crate::agent::process::ProcessError> {
    ProcessSession::spawn(
        ProcessSpec {
            program: "/bin/cat".into(),
            arguments: vec![],
            directory: path.into(),
            environment: BTreeMap::new(),
        },
        ProcessLimits {
            retained_bytes: 128,
            execution_timeout: Duration::from_secs(10),
            drain_timeout: Duration::from_secs(2),
        },
    )
}
#[tokio::test]
async fn normal_and_panicking_turns_close_admission_and_reap_owned_processes() {
    for panicking in [false, true] {
        let (_, _, service, id) = super::super::tests::setup();
        let service = Arc::new(service);
        let temp = tempfile::tempdir().unwrap();
        service.create(id).await.unwrap();
        let worker = service.clone();
        let (send, receive) = oneshot::channel();
        let owned = service
            .start_owned_turn(id, move |turn_id, _| async move {
                let scope = WorkspaceIdentity {
                    lease_id: Uuid::new_v4(),
                    conversation_id: *id.as_uuid(),
                    turn_id,
                    environment_id: "test".into(),
                };
                let process = worker
                    .processes()
                    .start(scope.clone(), || launch(temp.path()))
                    .unwrap();
                send.send((scope, process)).unwrap();
                assert!(!panicking, "synthetic turn panic after process launch");
                Ok(())
            })
            .unwrap();
        let (mut scope, process) = receive.await.unwrap();
        let completion = timeout(Duration::from_secs(4), owned.task.wait())
            .await
            .unwrap();
        assert_eq!(
            completion,
            if panicking {
                TaskCompletion::RecoveryRequired
            } else {
                TaskCompletion::Completed
            }
        );
        let ProcessPoll::Finished(result) = service
            .processes()
            .poll(&scope, process, Duration::ZERO)
            .await
            .unwrap()
        else {
            panic!("not finished");
        };
        assert_eq!(result.reason, StopReason::Cancelled);
        assert!(result.drained);
        service.processes().forget(&scope, process).unwrap();
        scope.lease_id = Uuid::new_v4();
        assert!(matches!(
            service
                .processes()
                .start(scope, || panic!("late launcher must not run")),
            Err(ProcessRegistryError::Closing)
        ));
    }
}
#[tokio::test]
async fn turn_cancel_wakes_process_wait_without_dropping_operation() {
    let (_, _, service, id) = super::super::tests::setup();
    let service = Arc::new(service);
    let worker = service.clone();
    let temp = tempfile::tempdir().unwrap();
    service.create(id).await.unwrap();
    let (send, receive) = oneshot::channel();
    let (finished, observed) = oneshot::channel();
    let owned = service
        .start_owned_turn(id, move |turn_id, signal| async move {
            let scope = WorkspaceIdentity {
                lease_id: Uuid::new_v4(),
                conversation_id: *id.as_uuid(),
                turn_id,
                environment_id: "test".into(),
            };
            let process = worker
                .processes()
                .start(scope.clone(), || launch(temp.path()))
                .unwrap();
            send.send(()).unwrap();
            let result = worker
                .processes()
                .poll(&scope, process, Duration::from_secs(5))
                .await
                .unwrap();
            assert!(matches!(result, ProcessPoll::Finished(_)));
            assert!(signal.is_cancelled());
            finished.send(()).unwrap();
            Err(research_agent_core::model::ModelError::Canceled.into())
        })
        .unwrap();
    receive.await.unwrap();
    service.tasks().cancel_turn(id, owned.turn_id).unwrap();
    assert_eq!(
        timeout(Duration::from_secs(4), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    observed.await.unwrap();
}
