use super::*;
use crate::agent::writing_submission::{WritingSubmissionError, workspace};
use crate::runtime_plugins::native::{NativeHostError, NativeProfile};

#[test]
fn initial_source_diagnostic_reuses_the_original_native_envelope_and_other_codes_keep_their_shape()
{
    for (error, expected) in [
        (Error::InvalidInput, "invalidInput"),
        (Error::Busy, "busy"),
        (Error::Conflict, "conflict"),
        (Error::NotFound, "notFound"),
        (Error::Denied, "denied"),
        (Error::Unavailable, "unavailable"),
        (Error::RecoveryRequired, "recoveryRequired"),
        (Error::UnsupportedPlatform, "unsupportedPlatform"),
    ] {
        assert_eq!(serde_json::to_value(error).unwrap(), expected);
    }
    let scope = Scope {
        conversation_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        manuscript_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
    };
    let mut cause = NativeHostError::for_profile(
        NativeProfile::WritingProjects,
        "plugin_host_source_rejected",
        "writing context source unavailable",
    )
    .package(&"a".repeat(64));
    let instance = Uuid::new_v4();
    let native_request = Uuid::new_v4();
    cause.instance_id = Some(instance.to_string());
    cause.request_id = Some(native_request.to_string());
    let mapped = preparation_error(
        workspace::Error::from(WritingSubmissionError::from(cause)),
        &scope,
    );
    let value = serde_json::to_value(mapped).unwrap();
    assert_eq!(value["code"], "plugin_host_source_rejected");
    assert_eq!(value["conversationId"], scope.conversation_id.to_string());
    assert_eq!(value["manuscriptId"], scope.manuscript_id.to_string());
    assert_eq!(value["operationId"], scope.request_id.to_string());
    assert_eq!(value["requestId"], native_request.to_string());
    assert_eq!(value["instanceId"], instance.to_string());
    assert_eq!(value["packageKey"], "a".repeat(64));
}
