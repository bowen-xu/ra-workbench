use super::*;
use crate::ai::structured_resource_validation::FailureCause;
use serde_json::json;

fn resource(error: Error, phase: &'static str, outcome: HbsOutcome) -> ResourceOperationError {
    OperationError::Resource(BoundaryFailure {
        failure: Failure {
            error,
            cause: FailureCause::Original,
        },
        phase,
        outcome,
        rollback_error: None,
    })
    .into_resource("core-review:1")
}

#[test]
fn committed_draft_decode_preserves_its_original_safe_cause_without_unknown_settlement() {
    let error = resource(
        Error::Invalid("结构化资源存储格式无效".into()),
        "postcommit-read",
        HbsOutcome::Committed,
    );
    assert_eq!(error.code, "storage_error");
    assert!(error.message.contains("结构化资源存储格式无效"));
    let diagnostic = error.native_failure.unwrap();
    assert_eq!(diagnostic.code, "invalid_input");
    assert_eq!(diagnostic.kind, "technical");
    assert_eq!(diagnostic.phase, "postcommit-read");
    assert_eq!(diagnostic.outcome, HbsOutcome::Committed);
    assert!(diagnostic.native_request_id.is_none());
    assert!(diagnostic.instance_id.is_none());
}

#[test]
fn arbitrary_postcommit_error_text_never_becomes_a_safe_native_cause() {
    let error = resource(
        Error::Invalid("select private_column from private_table; synthetic-value".into()),
        "postcommit-read",
        HbsOutcome::Committed,
    );
    let encoded = serde_json::to_value(error).unwrap();
    assert!(!encoded.to_string().contains("private_"));
    assert!(!encoded.to_string().contains("synthetic-value"));
    assert_eq!(encoded["nativeFailure"]["outcome"], "committed");
    assert_eq!(encoded["requestId"], "core-review:1");
}

#[test]
fn definite_core_rejection_has_only_original_code_message_and_actual_core_request() {
    let error = resource(Error::Conflict, "read", HbsOutcome::RolledBack);
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({
            "code":"revision_conflict",
            "message":"资源或草稿版本已变化，请重新读取后修改",
            "requestId":"core-review:1"
        })
    );
}

#[test]
fn write_technical_failure_describes_publication_without_claiming_the_stage_was_rolled_back() {
    let error = OperationError::Existing(HbsOperationError::native(
        crate::runtime_plugins::native::NativeHostError::for_profile(
            crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
            "plugin_host_exited",
            "untrusted private-body",
        ),
        "validation",
        HbsOutcome::RolledBack,
    ))
    .into_write("core-write:1");
    assert_eq!(error.code, "storage_error");
    assert!(error.message.contains("重试原保存请求"));
    assert!(!error.message.contains("private-body"));
    let encoded = serde_json::to_value(error).unwrap();
    assert_eq!(encoded["requestId"], "core-write:1");
    assert_eq!(encoded["nativeFailure"]["outcome"], "rolledBack");
    assert_eq!(encoded["nativeFailure"]["phase"], "validation");
    assert!(encoded["nativeFailure"].get("nativeRequestId").is_none());
}
