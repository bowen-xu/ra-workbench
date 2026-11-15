use super::*;
use crate::{agent::note_submission::source::Resource, runtime_plugins::native::NativeProfile};
#[cfg(target_os = "macos")]
use serde_json::json;

fn receipt() -> SourceReceipt {
    SourceReceipt {
        conversation_id: Uuid::from_u128(1),
        request_id: Uuid::from_u128(2),
        resource: Resource::Page(Uuid::from_u128(3)),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn forwards_confirmed_termination_metadata_without_replacing_receipt_or_native_identity() {
    use crate::agent::process::{ProcessError, ProcessIoOperation};

    // The host owns confirmation of drain/input cleanup. This unit only checks
    // the already-confirmed typed cause's projection through the Note boundary.
    let termination = NativeTerminationDiagnostic::from_initial_group_termination(
        &ProcessError::OperationIo {
            operation: ProcessIoOperation::GroupTerminate,
            kind: std::io::ErrorKind::PermissionDenied,
            raw_os_error: Some(nix::libc::EPERM),
        },
        false,
    )
    .unwrap();
    let source = receipt();
    let mut cause = NativePreviewError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "文献回复与本次请求不符。",
    );
    cause.package_key = Some("a".repeat(64));
    cause.instance_id = Some(Uuid::from_u128(4).to_string());
    cause.request_id = Some(Uuid::from_u128(5).to_string());
    cause.exit_code = Some(1);
    cause.termination_error = Some(termination.clone());
    let Error::Native(diagnostic) = host(source, "fallback-package", *cause) else {
        panic!("expected the original Note native diagnostic");
    };
    assert_eq!(diagnostic.termination_error.as_ref(), Some(&termination));
    assert_eq!(diagnostic.conversation_id, source.conversation_id);
    assert_eq!(diagnostic.request_id, source.request_id);
    assert_eq!(diagnostic.page_id, source.resource.id());
    assert_eq!(
        diagnostic.package_key.as_deref(),
        Some("a".repeat(64).as_str())
    );
    assert_eq!(diagnostic.instance_id, Some(Uuid::from_u128(4).to_string()));
    assert_eq!(
        diagnostic.native_request_id,
        Some(Uuid::from_u128(5).to_string())
    );
    assert_eq!(diagnostic.exit_code, Some(1));
    assert!(diagnostic.cleanup_error.is_none());
    let wire = serde_json::to_value(Error::Native(diagnostic)).unwrap();
    assert_eq!(wire["code"], "plugin_host_response_mismatch");
    assert_eq!(wire["message"], "文献回复与本次请求不符。");
    assert_eq!(wire["requestId"], source.request_id.to_string());
    assert_eq!(wire["nativeRequestId"], Uuid::from_u128(5).to_string());
    assert_eq!(
        wire["terminationError"],
        json!({
            "operation":"group_terminate","kind":"PermissionDenied","rawOsError":1
        })
    );
    assert!(wire.get("cleanupError").is_none());
}

#[test]
fn missing_termination_metadata_preserves_legacy_host_and_data_diagnostics() {
    let source = receipt();
    let cause = NativePreviewError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_exited",
        "文献宿主在回复前退出。",
    );
    let error = host(source, "legacy-package", *cause);
    let wire = serde_json::to_value(error).unwrap();
    assert_eq!(wire["code"], "plugin_host_exited");
    assert_eq!(wire["message"], "文献宿主在回复前退出。");
    assert_eq!(wire["conversationId"], source.conversation_id.to_string());
    assert_eq!(wire["requestId"], source.request_id.to_string());
    assert_eq!(wire["pageId"], source.resource.id().to_string());
    assert_eq!(wire["packageKey"], "legacy-package");
    assert!(wire.get("terminationError").is_none());
    assert!(wire.get("instanceId").is_none());
    assert!(wire.get("nativeRequestId").is_none());
    let data = consumption(source, "legacy-package", TraceDataError::InvalidObservation);
    let wire = serde_json::to_value(data).unwrap();
    assert_eq!(wire["code"], "plugin_data_result_invalid");
    assert!(wire.get("terminationError").is_none());
}
