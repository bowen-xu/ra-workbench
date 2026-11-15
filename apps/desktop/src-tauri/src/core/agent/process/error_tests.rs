use super::{ProcessError, ProcessIoOperation};
use std::io;

#[test]
fn operation_diagnostic_preserves_native_errno_including_zero() {
    for (operation, label) in [
        (ProcessIoOperation::GroupInterrupt, "group_interrupt"),
        (ProcessIoOperation::GroupTerminate, "group_terminate"),
        (ProcessIoOperation::ChildStartKill, "child_start_kill"),
        (ProcessIoOperation::ChildWait, "child_wait"),
        (
            ProcessIoOperation::GroupCleanupAfterWait,
            "group_cleanup_after_wait",
        ),
    ] {
        for code in [0, nix::libc::EPERM] {
            let native = io::Error::from_raw_os_error(code);
            let expected_kind = native.kind();
            let error = ProcessError::operation_io(operation, native);
            assert!(matches!(
                &error,
                ProcessError::OperationIo {
                    operation: actual_operation,
                    kind,
                    raw_os_error: Some(actual),
                } if *actual_operation == operation && *kind == expected_kind && *actual == code
            ));
            assert!(error.to_string().contains(&format!("operation={label}")));
            assert!(
                error
                    .to_string()
                    .contains(&format!("raw_os_error=Some({code})"))
            );
        }
    }
}

#[test]
fn missing_errno_is_not_invented_and_private_io_text_is_not_retained() {
    let private = "synthetic private command and /synthetic/private/path";
    let error = ProcessError::operation_io(
        ProcessIoOperation::ChildWait,
        io::Error::new(io::ErrorKind::PermissionDenied, private),
    );
    assert!(matches!(
        &error,
        ProcessError::OperationIo {
            operation: ProcessIoOperation::ChildWait,
            kind: io::ErrorKind::PermissionDenied,
            raw_os_error: None,
        }
    ));
    assert!(error.to_string().contains("raw_os_error=None"));
    assert!(!error.to_string().contains(private));
    assert!(!format!("{error:?}").contains(private));
}

#[test]
fn ordinary_io_conversion_keeps_the_original_kind_only_variant() {
    let error = ProcessError::from(io::Error::from_raw_os_error(nix::libc::ENOENT));
    assert!(matches!(&error, ProcessError::Io(io::ErrorKind::NotFound)));
    assert_eq!(error.to_string(), "process I/O failed (NotFound)");
}

#[cfg(target_os = "macos")]
#[test]
fn termination_recovery_keeps_both_fixed_safe_operation_failures() {
    for (recovery_operation, recovery_errno) in [
        (ProcessIoOperation::ChildWait, nix::libc::ECHILD),
        (ProcessIoOperation::GroupCleanupAfterWait, nix::libc::EPERM),
    ] {
        let error = ProcessError::termination_recovery(
            io::Error::from_raw_os_error(nix::libc::EPERM),
            recovery_operation,
            io::Error::from_raw_os_error(recovery_errno),
        );
        let ProcessError::TerminationRecovery { initial, recovery } = &error else {
            panic!("expected the fixed termination recovery pair");
        };
        assert_eq!(initial.operation, ProcessIoOperation::GroupTerminate);
        assert_eq!(initial.kind, io::ErrorKind::PermissionDenied);
        assert_eq!(initial.raw_os_error, Some(nix::libc::EPERM));
        assert_eq!(recovery.operation, recovery_operation);
        assert_eq!(recovery.raw_os_error, Some(recovery_errno));
        assert_eq!(
            recovery.kind,
            io::Error::from_raw_os_error(recovery_errno).kind()
        );
        let display = error.to_string();
        assert!(display.contains("operation=group_terminate"));
        assert!(display.contains(&format!("operation={recovery_operation}")));
        assert!(display.contains(&format!("raw_os_error=Some({})", nix::libc::EPERM)));
        assert!(display.contains(&format!("raw_os_error=Some({recovery_errno})")));
    }
    let private = "synthetic private command /synthetic/private/path";
    let error = ProcessError::termination_recovery(
        io::Error::new(io::ErrorKind::PermissionDenied, private),
        ProcessIoOperation::ChildWait,
        io::Error::other(private),
    );
    assert!(!format!("{error:?}").contains(private));
    assert!(!error.to_string().contains(private));
    assert!(error.to_string().contains("raw_os_error=None"));
}
