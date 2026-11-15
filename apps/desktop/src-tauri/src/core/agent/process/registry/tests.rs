use super::*;
use crate::agent::process::{ProcessLimits, ProcessSpec, StopReason};
use std::{collections::BTreeMap, path::Path};
use tokio::time::{sleep, timeout};
fn scope() -> WorkspaceIdentity {
    WorkspaceIdentity {
        lease_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        environment_id: "test".into(),
    }
}
fn launch(root: &Path) -> Result<ProcessSession, ProcessError> {
    ProcessSession::spawn(
        ProcessSpec {
            program: "/bin/cat".into(),
            arguments: vec![],
            directory: root.into(),
            environment: BTreeMap::new(),
        },
        ProcessLimits {
            retained_bytes: 128,
            execution_timeout: Duration::from_secs(5),
            drain_timeout: Duration::from_secs(2),
        },
    )
}
#[tokio::test]
async fn identity_checks_precede_reads_writes_and_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProcessRegistry::default();
    let owner = scope();
    registry
        .begin_turn(owner.conversation_id, owner.turn_id)
        .unwrap();
    let id = registry
        .start(owner.clone(), || launch(temp.path()))
        .unwrap();
    let mut wrong = owner.clone();
    wrong.turn_id = Uuid::new_v4();
    assert!(matches!(
        registry.write(&wrong, id, b"bad").await,
        Err(ProcessRegistryError::NotFound)
    ));
    assert!(matches!(
        registry.cancel(&wrong, id),
        Err(ProcessRegistryError::NotFound)
    ));
    assert!(matches!(
        registry.poll(&wrong, id, Duration::ZERO).await,
        Err(ProcessRegistryError::NotFound)
    ));
    assert!(matches!(
        registry.forget(&owner, id),
        Err(ProcessRegistryError::Busy)
    ));
    assert!(matches!(
        registry.start(owner.clone(), || panic!("must not launch duplicate lease")),
        Err(ProcessRegistryError::Busy)
    ));
    registry.write(&owner, id, b"kept\n").await.unwrap();
    registry.close_input(&owner, id).await.unwrap();
    let ProcessPoll::Finished(result) = registry
        .poll(&owner, id, Duration::from_secs(4))
        .await
        .unwrap()
    else {
        panic!("not finished");
    };
    assert!(result.stdout.head.starts_with(b"kept"));
    registry.forget(&owner, id).unwrap();
    assert!(matches!(
        registry.cancel(&owner, id),
        Err(ProcessRegistryError::NotFound)
    ));
}
#[tokio::test]
async fn cancel_is_not_blocked_by_a_waiter_and_does_not_cancel_other_turns() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProcessRegistry::default();
    let owner = scope();
    registry
        .begin_turn(owner.conversation_id, owner.turn_id)
        .unwrap();
    let other = scope();
    registry
        .begin_turn(other.conversation_id, other.turn_id)
        .unwrap();
    let id = registry
        .start(owner.clone(), || launch(temp.path()))
        .unwrap();
    let other_id = registry
        .start(other.clone(), || launch(temp.path()))
        .unwrap();
    let (result, ()) = timeout(Duration::from_secs(3), async {
        tokio::join!(registry.poll(&owner, id, Duration::from_secs(5)), async {
            sleep(Duration::from_millis(20)).await;
            registry
                .cancel_turn(owner.conversation_id, owner.turn_id)
                .unwrap();
        })
    })
    .await
    .unwrap();
    let ProcessPoll::Finished(result) = result.unwrap() else {
        panic!("not finished");
    };
    assert_eq!(result.reason, StopReason::Cancelled);
    assert!(matches!(
        registry
            .poll(&other, other_id, Duration::ZERO)
            .await
            .unwrap(),
        ProcessPoll::Running(_)
    ));
    registry.shutdown().await.unwrap();
    registry.shutdown().await.unwrap();
    assert!(matches!(
        registry.start(scope(), || panic!("closed admission")),
        Err(ProcessRegistryError::Closing)
    ));
}
#[tokio::test]
async fn capacity_is_checked_before_spawn_and_completed_handles_remain_counted() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProcessRegistry::default();
    let mut handles = vec![];
    for _ in 0..16 {
        let scope = scope();
        registry
            .begin_turn(scope.conversation_id, scope.turn_id)
            .unwrap();
        let id = registry
            .start(scope.clone(), || launch(temp.path()))
            .unwrap();
        handles.push((scope, id));
    }
    assert!(matches!(
        registry.start(scope(), || panic!("full admission")),
        Err(ProcessRegistryError::Capacity)
    ));
    let (owner, id) = &handles[0];
    registry.close_input(owner, *id).await.unwrap();
    assert!(matches!(
        registry
            .poll(owner, *id, Duration::from_secs(4))
            .await
            .unwrap(),
        ProcessPoll::Finished(_)
    ));
    assert!(matches!(
        registry.start(scope(), || panic!("completed still retained")),
        Err(ProcessRegistryError::Capacity)
    ));
    registry.forget(owner, *id).unwrap();
    let fresh = scope();
    registry
        .begin_turn(fresh.conversation_id, fresh.turn_id)
        .unwrap();
    registry.start(fresh, || launch(temp.path())).unwrap();
    registry.shutdown().await.unwrap();
}
