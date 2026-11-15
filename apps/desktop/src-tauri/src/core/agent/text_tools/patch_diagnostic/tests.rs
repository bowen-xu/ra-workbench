use super::*;
use research_agent_core::{protocol::ModelIdentity, submission::SubmissionInput};

fn record() -> SubmissionRecord {
    SubmissionRecord {
        conversation_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        input: SubmissionInput {
            remote_compaction_v2: None,
            text: "private-patch-body".into(),
            user_input: vec![],
            skill_instructions: vec![],
            model: ModelIdentity {
                provider_id: "synthetic".into(),
                model_id: "synthetic".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        },
    }
}
fn detail(record: &SubmissionRecord) -> Value {
    let key = format!("{}:native-call", record.request_id);
    let error = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(1811),
        Some("private-sql-message".into()),
    );
    patch_journal::call_diagnostic(record, &key, "call-terminal", &error)
}
fn structured(error: RunError) -> Value {
    match error {
        RunError::ToolOutcomeUnknownDiagnostic(detail) => serde_json::to_value(detail).unwrap(),
        _ => panic!("safe native diagnostic must remain structured"),
    }
}

#[test]
fn known_sql_metadata_crosses_unknown_error_without_copying_patch_or_diff_payloads() {
    let record = record();
    let detail = detail(&record);
    let value = json!({"status":"unknown","storageFailure":detail,"targets":[],"fileDiffs":[{"content":"private-file-diff"}],"patch":"private-patch-body"});
    let result = structured(outcome_error(&record, &value, &[]));
    assert_eq!(result["code"], "TEXT_PATCH_JOURNAL_UNCONFIRMED");
    assert_eq!(result["requestId"], json!(record.request_id));
    assert_eq!(result["turnId"], json!(record.turn_id));
    assert_eq!(
        result["contextSnapshotId"],
        json!(record.context_snapshot_id)
    );
    assert_eq!(result["phase"], "call-terminal");
    assert_eq!(result["sqliteCode"], "ConstraintViolation");
    assert_eq!(result["sqliteExtendedCode"], 1811);
    assert!(!result.to_string().contains("private-"));
    assert!(result.get("epoch").is_none());
    assert!(result.get("sqlFence").is_none());
}

#[test]
fn unsafe_provider_call_identifiers_do_not_discard_known_scope_or_sql_cause() {
    let record = record();
    let mut detail = detail(&record);
    for key in [
        "callId",
        "callRequestId",
        "targetRequestId",
        "journalRequestId",
    ] {
        detail[key] = json!("unsafe provider\nprivate-body");
    }
    let result = structured(detail_error(&record, &detail));
    for key in [
        "callId",
        "callRequestId",
        "targetRequestId",
        "journalRequestId",
    ] {
        assert!(result.get(key).is_none());
    }
    assert_eq!(result["conversationId"], json!(record.conversation_id));
    assert_eq!(result["requestId"], json!(record.request_id));
    assert_eq!(result["sqliteExtendedCode"], 1811);
    assert!(!result.to_string().contains("private-body"));
}

#[test]
fn absent_foreign_or_nonproducer_metadata_preserves_the_compatible_unit_unknown() {
    let record = record();
    let original = detail(&record);
    let mut foreign = original.clone();
    foreign["requestId"] = json!(Uuid::new_v4());
    let mut extra = original.clone();
    extra["rawSql"] = json!("private-sql-message");
    let mut cause = original.clone();
    cause["safeCause"] = json!("unowned_cause");
    for detail in [Value::Null, foreign, extra, cause] {
        assert!(matches!(
            detail_error(&record, &detail),
            RunError::ToolOutcomeUnknown
        ));
    }
    assert!(matches!(
        outcome_error(
            &record,
            &json!({"targets":[],"body":"private-patch-body"}),
            &[]
        ),
        RunError::ToolOutcomeUnknown
    ));
}

#[test]
fn only_owned_unknown_target_diagnostics_are_lifted_from_target_results() {
    let record = record();
    let detail = detail(&record);
    let mut target = TargetResult {
        path: "source://synthetic".into(),
        name: None,
        status: TargetStatus::Unknown,
        message: detail.to_string(),
        change_id: None,
        generation: None,
    };
    let result = structured(outcome_error(&record, &json!({}), &[target.clone()]));
    assert_eq!(result["sqliteExtendedCode"], 1811);
    target.status = TargetStatus::Applied;
    assert!(matches!(
        outcome_error(&record, &json!({}), &[target.clone()]),
        RunError::ToolOutcomeUnknown
    ));
    target.status = TargetStatus::Unknown;
    target.message = "private-sql-message".into();
    assert!(matches!(
        outcome_error(&record, &json!({}), &[target]),
        RunError::ToolOutcomeUnknown
    ));
}

#[test]
fn structured_query_keeps_only_the_matching_original_producer_phase_and_cause() {
    let record = record();
    let key = format!("{}:native-call:file:0", record.request_id);
    let mut value = detail(&record);
    value["code"] = json!("TEXT_PATCH_EFFECT_UNCONFIRMED");
    value["phase"] = json!("structured-query");
    value["safeCause"] = json!("structured_effect_unconfirmed");
    value["targetRequestId"] = json!(key);
    let retained = patch_journal::unavailable_receipt(&record, &key, Some(&value.to_string()));
    assert_eq!(retained["phase"], "structured-query");
    assert_eq!(retained["sqliteExtendedCode"], 1811);
    assert!(query_detail(&record, "foreign-target", &value).is_none());
    let mut conflicting = value.clone();
    conflicting["targetRequestId"] = json!("foreign-target");
    conflicting["journalRequestId"] = json!(key);
    assert!(query_detail(&record, &key, &conflicting).is_none());
    let mut mismatched = value.clone();
    mismatched["phase"] = json!("call-terminal");
    assert!(query_detail(&record, &key, &mismatched).is_none());
    value["safeCause"] = json!("journal_sql_failed");
    assert!(query_detail(&record, &key, &value).is_none());
}
