//! Focused real-process cleanup contract, not a deterministic kernel-race replay.
//! Only the initial terminate result is injected. Draining, both pipe EOFs,
//! original child/group settlement and InputLease unlink remain production code.
use super::*;
use crate::agent::process::{ProcessError, ProcessIoOperation};
use serde_json::{Value, json};
use std::{fs, io};

mod fixture;
use fixture::Fixture;

const WAIT: Duration = Duration::from_secs(8);

impl crate::runtime_plugins::native::NativePreviewService {
    pub(crate) async fn terminate_active_host_for_test(&self) -> String {
        let mut slot = self.host.lock().await;
        let host = slot.as_mut().expect("actual writing host is initialized");
        let instance = host.instance_id.clone();
        assert!(host.process.exit().is_none());
        host.process.terminate().unwrap();
        timeout(WAIT, async {
            while host.process.next_output().await.unwrap().is_some() {}
        })
        .await
        .unwrap();
        assert_eq!(
            host.process.exit().unwrap().signal,
            Some(nix::libc::SIGKILL)
        );
        instance
    }
}

fn assert_recovered_diagnostic(error: &NativeHostError) {
    assert!(error.cleanup_error.is_none(), "safe diagnostic: {error:?}");
    assert_eq!(
        serde_json::to_value(error).unwrap()["terminationError"],
        json!({"operation":"group_terminate","kind":"PermissionDenied","rawOsError":1})
    );
}

fn assert_primary(error: &NativeHostError, identity: &(String, String), key: &str) {
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.message, "Synthetic original binding failure");
    assert_eq!(error.package_key.as_deref(), Some(key));
    assert_eq!(error.instance_id.as_deref(), Some(identity.0.as_str()));
    assert_eq!(error.request_id.as_deref(), Some(identity.1.as_str()));
}

#[tokio::test]
async fn initial_group_eperm_is_retained_separately_only_after_actual_complete_drain() {
    let mut fixture = Fixture::new(false).await;
    let identity = fixture.identity();
    let key = fixture.host.package_key().to_owned();
    let request = fixture.request_bytes();
    let mut error = fixture.primary();
    assert!(fixture.host.process.exit().is_none());
    fixture.inject(ProcessIoOperation::GroupTerminate);

    timeout(WAIT, fixture.host.finish_failed(&mut error))
        .await
        .unwrap();
    assert_primary(&error, &identity, &key);
    assert_recovered_diagnostic(&error);
    assert!(fixture.host.initial_termination_error.is_none());
    fixture.assert_drained().await;
    assert_eq!(error.exit_code, Some(0));
    assert_eq!(fixture.request_bytes(), request);
    assert!(fixture.host.input_lease.is_none());
    assert!(fixture.host.input_cleanup.failure().is_none());
    let directory = fixture.directory.clone();
    let executable = fixture.executable.clone();
    drop(fixture);
    assert!(!directory.exists() && !executable.exists());
}

#[tokio::test]
async fn real_drain_timeout_keeps_the_initial_eperm_as_an_unconfirmed_failure() {
    let mut fixture = Fixture::new(true).await;
    let identity = fixture.identity();
    let key = fixture.host.package_key().to_owned();
    let request = fixture.request_bytes();
    let mut error = fixture.primary();
    fixture.inject(ProcessIoOperation::GroupTerminate);

    timeout(WAIT, fixture.host.finish_failed(&mut error))
        .await
        .unwrap();
    assert_primary(&error, &identity, &key);
    let cause = error.cleanup_error.as_deref().unwrap();
    assert!(cause.contains("operation=group_terminate") && cause.contains("raw_os_error=Some(1)"));
    assert!(cause.contains("plugin_host_cleanup_timeout"));
    assert!(error.termination_error.is_none());
    assert!(fixture.host.process.exit().is_none());
    let blocked = fixture
        .host
        .decide_note_trace(fixture::input(), WAIT)
        .await
        .unwrap_err();
    assert_eq!(blocked.code, "plugin_host_connection_failed");
    assert_eq!(fixture.request_bytes(), request);

    // One explicit FIFO release changes the real child state. We do not retry
    // a flaky assertion, erase the timeout or substitute an observed exit.
    fixture.release_exit();
    // The released child emits its known final stderr before exit. Actually
    // drain that tail before asserting the same owner's final None/reap.
    timeout(WAIT, fixture.host.drain(DrainMode::Cleanup))
        .await
        .unwrap()
        .unwrap();
    fixture.assert_drained().await;
    assert!(
        error
            .cleanup_error
            .as_deref()
            .unwrap()
            .contains("plugin_host_cleanup_timeout")
    );
    let directory = fixture.directory.clone();
    let executable = fixture.executable.clone();
    drop(fixture);
    assert!(!directory.exists() && !executable.exists());
}

#[tokio::test]
async fn a_later_group_cleanup_permission_cause_is_not_erased_by_real_drain_success() {
    let mut fixture = Fixture::new(false).await;
    let mut error = fixture.primary();
    fixture.inject(ProcessIoOperation::GroupCleanupAfterWait);

    timeout(WAIT, fixture.host.finish_failed(&mut error))
        .await
        .unwrap();
    fixture.assert_drained().await;
    assert!(error.termination_error.is_none());
    let cause = error.cleanup_error.as_deref().unwrap();
    assert!(cause.contains("operation=group_cleanup_after_wait"));
    assert!(cause.contains("PermissionDenied") && cause.contains("raw_os_error=Some(1)"));
    assert_eq!(error.code, "plugin_host_response_mismatch");
}

#[tokio::test]
async fn actual_input_unlink_failure_blocks_even_when_the_same_process_reaps() {
    let mut fixture = Fixture::new(false).await;
    let blocker = fixture.obstruct_input_unlink();
    let identity = fixture.identity();
    let key = fixture.host.package_key().to_owned();
    let mut error = fixture.primary();
    fixture.inject(ProcessIoOperation::GroupTerminate);

    timeout(WAIT, fixture.host.finish_failed(&mut error))
        .await
        .unwrap();
    assert_primary(&error, &identity, &key);
    fixture.assert_drained().await;
    assert!(error.termination_error.is_none());
    let cause = error.cleanup_error.as_deref().unwrap();
    assert!(cause.contains("原生输入清理失败") && cause.contains("group_terminate"));
    assert!(blocker.is_dir());
    assert!(fixture.host.input_cleanup.failure().is_some());
    let shutdown = timeout(WAIT, fixture.host.shutdown())
        .await
        .unwrap()
        .unwrap_err();
    assert!(shutdown.cleanup_error.is_some());
    assert_eq!(shutdown.code, "plugin_host_connection_failed");
    fs::remove_dir(blocker).unwrap();
}

#[tokio::test]
async fn failed_shutdown_returns_the_recovered_diagnostic_without_reusing_or_replaying() {
    let mut fixture = Fixture::new(false).await;
    let identity = fixture.identity();
    let request = fixture.request_bytes();
    fixture.host.failed = true;
    fixture.inject(ProcessIoOperation::GroupTerminate);

    let error = timeout(WAIT, fixture.host.shutdown())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_connection_failed");
    assert_eq!(error.instance_id.as_deref(), Some(identity.0.as_str()));
    assert_eq!(error.request_id.as_deref(), Some(identity.1.as_str()));
    assert_recovered_diagnostic(&error);
    fixture.assert_drained().await;
    let blocked = fixture
        .host
        .decide_note_trace(fixture::input(), WAIT)
        .await
        .unwrap_err();
    assert_eq!(blocked.code, "plugin_host_connection_failed");
    assert_eq!(fixture.request_bytes(), request);

    // Explicitly create another host from the same accepted package. This is
    // not a claim about the Service's private replacement latch or an automatic
    // replay: each real fixture exchanged exactly its own one trace request.
    let (mut replacement, directory, executable) = fixture.replacement().await;
    let new_identity = replacement.request_identity().unwrap();
    assert_ne!(new_identity.0, identity.0);
    assert_ne!(new_identity.1, identity.1);
    let request: Value =
        serde_json::from_slice(&fs::read(directory.join("business-request")).unwrap()).unwrap();
    assert_eq!(request["instanceId"], new_identity.0);
    assert_eq!(request["requestId"], new_identity.1);
    timeout(WAIT, replacement.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(replacement.process.next_output().await.unwrap().is_none());
    drop(replacement);
    assert!(!directory.exists() && !executable.exists());
}

#[tokio::test]
async fn an_existing_unconfirmed_cleanup_cause_is_never_offset_by_later_success() {
    let mut fixture = Fixture::new(false).await;
    let mut error = fixture.primary();
    error.add_cleanup("Synthetic previously unconfirmed input writer");
    fixture.inject(ProcessIoOperation::GroupTerminate);

    timeout(WAIT, fixture.host.finish_failed(&mut error))
        .await
        .unwrap();
    fixture.assert_drained().await;
    assert!(error.termination_error.is_none());
    let cause = error.cleanup_error.as_deref().unwrap();
    assert!(cause.contains("Synthetic previously unconfirmed input writer"));
    assert!(cause.contains("group_terminate"));
}

#[tokio::test]
async fn an_actual_cached_exit_does_not_qualify_for_the_initial_group_exception() {
    let mut fixture = Fixture::new(false).await;
    fixture.host.input.close();
    timeout(WAIT, fixture.host.drain(DrainMode::Cleanup))
        .await
        .unwrap()
        .unwrap();
    fixture.assert_drained().await;
    let mut error = fixture.primary();
    fixture.inject(ProcessIoOperation::GroupTerminate);

    timeout(WAIT, fixture.host.finish_failed(&mut error))
        .await
        .unwrap();
    assert!(error.termination_error.is_none());
    assert!(
        error
            .cleanup_error
            .as_deref()
            .unwrap()
            .contains("group_terminate")
    );
    assert_eq!(error.exit_code, Some(0));
    fixture.assert_drained().await;
}

mod service_cancellation;
