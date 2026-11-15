//! Carry only the patch owner's safe metadata through the existing recovery error.
use super::*;
use research_agent_core::runner::ToolOutcomeDiagnostic;

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.:/".contains(&byte))
}
fn owned_detail(record: &SubmissionRecord, value: &Value) -> Option<RunError> {
    let delivery = value["code"] == "TEXT_PATCH_DELIVERY_UNCONFIRMED";
    let delivery_phase = matches!(
        value["phase"].as_str(),
        Some("note-delivery-read" | "note-delivery-publish")
    );
    let delivery_cause = matches!(
        value["safeCause"].as_str(),
        Some(
            "note_delivery_read_failed"
                | "note_delivery_owner_unavailable"
                | "note_delivery_receipt_missing"
                | "note_delivery_snapshot_invalid"
                | "note_delivery_publish_failed"
                | "note_delivery_transaction_unconfirmed"
        )
    );
    if delivery != delivery_phase || delivery != delivery_cause {
        return None;
    }
    let effect = value["code"] == "TEXT_PATCH_EFFECT_UNCONFIRMED";
    let effect_phase = matches!(
        value["phase"].as_str(),
        Some(
            "structured-create"
                | "structured-read"
                | "structured-write"
                | "structured-scope"
                | "structured-query"
        )
    );
    if effect != effect_phase || effect != (value["safeCause"] == "structured_effect_unconfirmed") {
        return None;
    }
    if !matches!(
        value["code"].as_str(),
        Some(
            "TEXT_PATCH_JOURNAL_UNCONFIRMED"
                | "TEXT_PATCH_EFFECT_UNCONFIRMED"
                | "TEXT_PATCH_DELIVERY_UNCONFIRMED"
        )
    ) || value["conversationId"] != json!(record.conversation_id)
        || value["requestId"] != json!(record.request_id)
        || !matches!(
            value["phase"].as_str(),
            Some(
                "target-progress"
                    | "target-terminal"
                    | "target-query"
                    | "call-progress"
                    | "call-terminal"
                    | "call-recovery"
                    | "journal-begin-read"
                    | "legacy-terminal"
                    | "legacy-recovery"
                    | "structured-create"
                    | "structured-read"
                    | "structured-write"
                    | "structured-scope"
                    | "structured-query"
                    | "note-delivery-read"
                    | "note-delivery-publish"
                    | "note-begin"
                    | "note-commit"
                    | "note-rollback"
            )
        )
        || !matches!(
            value["safeCause"].as_str(),
            Some(
                "journal_receipt_missing"
                    | "journal_sql_failed"
                    | "journal_owner_unavailable"
                    | "target_receipt_unavailable"
                    | "journal_result_invalid"
                    | "structured_effect_unconfirmed"
                    | "note_delivery_read_failed"
                    | "note_delivery_owner_unavailable"
                    | "note_delivery_receipt_missing"
                    | "note_delivery_snapshot_invalid"
                    | "note_delivery_publish_failed"
                    | "note_delivery_transaction_unconfirmed"
                    | "note_transaction_unconfirmed"
            )
        )
    {
        return None;
    }
    let original = value.as_object()?;
    if original.keys().any(|key| {
        !matches!(
            key.as_str(),
            "code"
                | "conversationId"
                | "requestId"
                | "turnId"
                | "contextSnapshotId"
                | "callId"
                | "callRequestId"
                | "targetRequestId"
                | "journalRequestId"
                | "phase"
                | "sqliteCode"
                | "sqliteExtendedCode"
                | "safeCause"
        )
    }) {
        return None;
    }
    let mut fields = original.clone();
    for (key, known) in [
        ("turnId", record.turn_id),
        ("contextSnapshotId", record.context_snapshot_id),
    ] {
        if fields
            .get(key)
            .is_some_and(|value| !value.is_null() && value != &json!(known))
        {
            return None;
        }
        fields.entry(key).or_insert_with(|| json!(known));
    }
    // Provider call IDs are opaque. Remove an unrepresentable optional ID while
    // retaining the original known scope, cause and numeric SQLite evidence.
    for key in [
        "callId",
        "callRequestId",
        "targetRequestId",
        "journalRequestId",
    ] {
        if fields
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !identifier(value))
        {
            fields.remove(key);
        }
    }
    ToolOutcomeDiagnostic::try_from_value(Value::Object(fields))
        .ok()
        .map(|detail| RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail)))
}

pub(super) fn detail_error(record: &SubmissionRecord, value: &Value) -> RunError {
    owned_detail(record, value).unwrap_or(RunError::ToolOutcomeUnknown)
}

pub(super) fn query_detail(record: &SubmissionRecord, key: &str, value: &Value) -> Option<Value> {
    let target = value
        .get("targetRequestId")
        .filter(|value| !value.is_null());
    if target.map_or(value["journalRequestId"] != key, |target| target != key) {
        return None;
    }
    match owned_detail(record, value)? {
        RunError::ToolOutcomeUnknownDiagnostic(detail) => serde_json::to_value(detail).ok(),
        _ => None,
    }
}
pub(super) fn outcome_error(
    record: &SubmissionRecord,
    result: &Value,
    targets: &[TargetResult],
) -> RunError {
    if let Some(error) = owned_detail(record, &result["storageFailure"]) {
        return error;
    }
    if let Some(details) = result["additionalStorageFailures"].as_array() {
        for detail in details {
            if let Some(error) = owned_detail(record, detail) {
                return error;
            }
        }
    }
    for target in targets
        .iter()
        .filter(|target| target.status == TargetStatus::Unknown)
    {
        if target.message.len() <= 8 * 1024
            && let Ok(detail) = serde_json::from_str::<Value>(&target.message)
            && let Some(error) = owned_detail(record, &detail)
        {
            return error;
        }
    }
    RunError::ToolOutcomeUnknown
}

#[cfg(test)]
#[path = "patch_diagnostic/tests.rs"]
mod tests;
