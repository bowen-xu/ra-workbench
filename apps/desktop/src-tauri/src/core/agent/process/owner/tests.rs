//! This verifies only the native error mapping, not real process reaping.
use super::*;

#[test]
fn group_signal_keeps_permission_failure_and_only_original_esrch_success() {
    assert!(group_signal_result(Ok(())).is_ok());
    assert!(group_signal_result(Err(Errno::ESRCH)).is_ok());
    let error = group_signal_result(Err(Errno::EPERM)).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(nix::libc::EPERM));
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}
