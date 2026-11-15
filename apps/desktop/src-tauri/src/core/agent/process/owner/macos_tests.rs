//! Real owned-child regressions for Darwin's zombie-only process-group result.
use super::*;
#[path = "macos_tests/fixture.rs"]
mod fixture;
use fixture::*;
use tokio::time::timeout;

#[tokio::test]
async fn termination_reaps_a_known_zombie_before_retrying_its_group() {
    let child = spawn(0).await;
    let mut owner = ProcessOwner::new(child.child).unwrap();
    drop(child.input);
    observe_exit_without_reap(child.pid).await;
    assert_eq!(killpg(child.pid, Signal::SIGKILL), Err(Errno::EPERM));

    owner.terminate().unwrap();
    assert_eq!(owner.exit(), Some(natural_exit()));
    assert!(owner.group.0.is_none());
    timeout(WAIT, owner.wait()).await.unwrap().unwrap();
}

#[tokio::test]
async fn a_permission_failure_with_a_live_owned_child_is_not_success() {
    let child = spawn(0).await;
    let mut owner = ProcessOwner::new(child.child).unwrap();
    // Only the initial syscall result is injected; the production recovery
    // branch must actually observe that this owned child has not exited.
    let error = owner
        .group_termination_failed(io::Error::from_raw_os_error(nix::libc::EPERM))
        .unwrap_err();
    assert_denied(&error, ProcessIoOperation::GroupTerminate);
    assert!(owner.exit().is_none());
    assert_eq!(owner.group.0, Some(child.pid));
    assert!(owner.child.try_wait().unwrap().is_none());

    owner.terminate().unwrap();
    timeout(WAIT, owner.wait()).await.unwrap().unwrap();
    assert_eq!(owner.exit().unwrap().signal, Some(9));
    drop(child.input);
}

#[tokio::test]
async fn a_zombie_leader_does_not_spare_its_live_owned_sibling() {
    let root = spawn(0).await;
    let mut sibling = spawn(root.pid.as_raw()).await;
    let mut owner = ProcessOwner::new(root.child).unwrap();
    drop(root.input);
    observe_exit_without_reap(root.pid).await;
    assert!(sibling.child.try_wait().unwrap().is_none());

    owner.terminate().unwrap();
    timeout(WAIT, owner.wait()).await.unwrap().unwrap();
    assert_eq!(owner.exit(), Some(natural_exit()));
    let status = timeout(WAIT, sibling.child.wait()).await.unwrap().unwrap();
    assert_eq!(status.signal(), Some(9));
    assert!(owner.group.0.is_none());
}

#[tokio::test]
async fn cached_root_exit_still_requires_the_pending_live_group_cleanup() {
    let root = spawn(0).await;
    let mut sibling = spawn(root.pid.as_raw()).await;
    let mut owner = ProcessOwner::new(root.child).unwrap();
    drop(root.input);
    observe_exit_without_reap(root.pid).await;
    let status = timeout(WAIT, owner.child.wait()).await.unwrap().unwrap();
    owner.exit = Some(ProcessExit {
        code: status.code(),
        signal: status.signal(),
    });
    assert_eq!(owner.group.0, Some(root.pid));

    // A previously cached root exit must not authorize another termination
    // recovery attempt, which would otherwise kill the still-live sibling here.
    let error = owner
        .group_termination_failed(io::Error::from_raw_os_error(nix::libc::EPERM))
        .unwrap_err();
    assert_denied(&error, ProcessIoOperation::GroupTerminate);
    assert!(sibling.child.try_wait().unwrap().is_none());

    timeout(WAIT, owner.wait()).await.unwrap().unwrap();
    let status = timeout(WAIT, sibling.child.wait()).await.unwrap().unwrap();
    assert_eq!(status.signal(), Some(9));
    assert_eq!(owner.exit(), Some(natural_exit()));
    assert!(owner.group.0.is_none());
}

#[tokio::test]
async fn cached_exit_keeps_failed_group_cleanup_pending_until_confirmed() {
    let root = spawn(0).await;
    let mut sibling = spawn(root.pid.as_raw()).await;
    let mut owner = ProcessOwner::new(root.child).unwrap();
    drop(root.input);
    observe_exit_without_reap(root.pid).await;
    drop(sibling.input.take());
    observe_exit_without_reap(sibling.pid).await;

    // Both children are unreaped zombies. The production recovery reaps only
    // its root; the sibling keeps the same group present for the second EPERM.
    assert!(owner.exit().is_none());
    let error = owner.terminate().unwrap_err();
    let ProcessError::TerminationRecovery { initial, recovery } = &error else {
        panic!("expected both real group termination failures: {error:?}");
    };
    assert_eq!(initial.operation, ProcessIoOperation::GroupTerminate);
    assert_eq!(
        recovery.operation,
        ProcessIoOperation::GroupCleanupAfterWait
    );
    for failure in [initial, recovery] {
        assert_eq!(failure.kind, io::ErrorKind::PermissionDenied);
        assert_eq!(failure.raw_os_error, Some(nix::libc::EPERM));
    }
    assert_eq!(owner.group.0, Some(root.pid));
    assert_eq!(owner.exit(), Some(natural_exit()));

    let error = timeout(WAIT, owner.wait()).await.unwrap().unwrap_err();
    assert_denied(&error, ProcessIoOperation::GroupCleanupAfterWait);
    assert_eq!(owner.group.0, Some(root.pid));
    assert_eq!(owner.exit(), Some(natural_exit()));

    // This explicit observation changes the known state before the second
    // cleanup call; it is not a retry of a flaky assertion or a timed loop.
    let status = sibling.child.try_wait().unwrap().unwrap();
    assert_eq!(status.code(), Some(1));
    timeout(WAIT, owner.wait()).await.unwrap().unwrap();
    assert!(owner.group.0.is_none());
    assert_eq!(owner.exit(), Some(natural_exit()));
}
