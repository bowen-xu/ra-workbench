use super::*;
use crate::agent::process::ProcessIoOperation;
use std::io::ErrorKind;

fn failure(
    operation: ProcessIoOperation,
    kind: ErrorKind,
    raw_os_error: Option<i32>,
) -> ProcessError {
    ProcessError::OperationIo {
        operation,
        kind,
        raw_os_error,
    }
}

#[test]
fn only_the_initial_uncached_macos_group_permission_error_has_the_fixed_diagnostic() {
    let initial = failure(
        ProcessIoOperation::GroupTerminate,
        ErrorKind::PermissionDenied,
        Some(nix::libc::EPERM),
    );
    let diagnostic =
        NativeTerminationDiagnostic::from_initial_group_termination(&initial, false).unwrap();
    assert_eq!(
        serde_json::to_value(diagnostic).unwrap(),
        serde_json::json!({
            "operation": "group_terminate",
            "kind": "PermissionDenied",
            "rawOsError": 1,
        })
    );
    assert!(NativeTerminationDiagnostic::from_initial_group_termination(&initial, true).is_none());
    for cause in [
        failure(
            ProcessIoOperation::GroupCleanupAfterWait,
            ErrorKind::PermissionDenied,
            Some(1),
        ),
        failure(
            ProcessIoOperation::ChildWait,
            ErrorKind::PermissionDenied,
            Some(1),
        ),
        failure(
            ProcessIoOperation::GroupTerminate,
            ErrorKind::Other,
            Some(1),
        ),
        failure(
            ProcessIoOperation::GroupTerminate,
            ErrorKind::PermissionDenied,
            None,
        ),
        failure(
            ProcessIoOperation::GroupTerminate,
            ErrorKind::PermissionDenied,
            Some(0),
        ),
        ProcessError::Io(ErrorKind::PermissionDenied),
    ] {
        assert!(
            NativeTerminationDiagnostic::from_initial_group_termination(&cause, false).is_none(),
            "must remain a cleanup failure: {cause:?}"
        );
    }
}

#[test]
fn legacy_error_json_omits_the_new_field_and_confirmed_diagnostics_keep_original_identities() {
    let mut error = NativeHostError::new("plugin_host_response_mismatch", "Synthetic mismatch");
    let original = serde_json::to_value(&error).unwrap();
    assert!(original.get("terminationError").is_none());
    error.instance_id = Some("00000000-0000-4000-8000-000000000001".into());
    error.request_id = Some("00000000-0000-4000-8000-000000000002".into());
    error.termination_error = NativeTerminationDiagnostic::from_initial_group_termination(
        &failure(
            ProcessIoOperation::GroupTerminate,
            ErrorKind::PermissionDenied,
            Some(1),
        ),
        false,
    );
    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["code"], original["code"]);
    assert_eq!(value["message"], original["message"]);
    assert_eq!(value["instanceId"], error.instance_id.as_deref().unwrap());
    assert_eq!(value["requestId"], error.request_id.as_deref().unwrap());
    assert_eq!(value["terminationError"]["rawOsError"], 1);
    assert!(value.get("cleanupError").is_none());
}
