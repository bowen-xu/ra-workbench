use super::*;
use futures_util::poll;

#[tokio::test]
async fn cancelling_pending_cleanup_fences_the_service() {
    let failed = AtomicBool::new(false);
    let mut cleanup = Box::pin(async {
        let guard = CleanupGuard::new(&failed);
        std::future::pending::<()>().await;
        guard.finish(None);
    });
    assert!(poll!(&mut cleanup).is_pending());
    assert!(!failed.load(Ordering::Acquire));
    drop(cleanup);
    assert!(failed.load(Ordering::Acquire));
}

#[test]
fn confirmed_cleanup_never_clears_an_existing_failure() {
    for previously_failed in [false, true] {
        let failed = AtomicBool::new(previously_failed);
        CleanupGuard::new(&failed).finish(None);
        assert_eq!(failed.load(Ordering::Acquire), previously_failed);
    }
}

#[test]
fn an_unconfirmed_cleanup_error_fences_the_service() {
    let failed = AtomicBool::new(false);
    let mut error = NativeHostError::new("plugin_host_cleanup_failed", "Synthetic cleanup failure");
    error.add_cleanup("process resources could not be released safely");
    CleanupGuard::new(&failed).finish(Some(error.as_ref()));
    assert!(failed.load(Ordering::Acquire));
}

#[cfg(target_os = "macos")]
#[test]
fn a_retained_termination_diagnostic_does_not_fence_confirmed_cleanup() {
    use crate::{
        agent::process::{ProcessError, ProcessIoOperation},
        runtime_plugins::native::NativeTerminationDiagnostic,
    };
    let failed = AtomicBool::new(false);
    let cause = ProcessError::OperationIo {
        operation: ProcessIoOperation::GroupTerminate,
        kind: std::io::ErrorKind::PermissionDenied,
        raw_os_error: Some(nix::libc::EPERM),
    };
    let diagnostic =
        NativeTerminationDiagnostic::from_initial_group_termination(&cause, false).unwrap();
    let mut error = NativeHostError::new("plugin_host_response_mismatch", "Synthetic rejection");
    error.termination_error = Some(diagnostic.clone());
    CleanupGuard::new(&failed).finish(Some(error.as_ref()));
    assert!(!failed.load(Ordering::Acquire));
    assert_eq!(error.termination_error, Some(diagnostic));
    assert!(error.cleanup_error.is_none());
}
