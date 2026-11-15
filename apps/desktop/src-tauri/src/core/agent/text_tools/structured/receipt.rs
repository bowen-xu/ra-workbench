//! Safe effect metadata; no provider text, content or raw SQL cause crosses this seam.
use super::*;

pub(super) fn uncertain(
    record: &SubmissionRecord,
    key: &str,
    phase: &'static str,
    error: Option<&crate::structured_resources::Error>,
) -> Value {
    let mut value = match error {
        Some(crate::structured_resources::Error::Database(error)) => {
            patch_journal::target_diagnostic(record, key, phase, error)
        }
        _ => patch_journal::unavailable_receipt(record, key, None),
    };
    value["code"] = json!("TEXT_PATCH_EFFECT_UNCONFIRMED");
    value["phase"] = json!(phase);
    value["safeCause"] = json!("structured_effect_unconfirmed");
    value
}

pub(super) fn append_unknown(results: &mut Vec<TargetResult>, path: String, detail: Value) {
    results.push(TargetResult {
        path,
        name: None,
        status: TargetStatus::Unknown,
        message: detail.to_string(),
        change_id: None,
        generation: None,
    });
}

pub(super) fn lost_receipt(results: &mut [TargetResult], detail: &Value) {
    // Invalid JSON also has a durable raw draft. Its failed validation result
    // cannot stand in for an acknowledged receipt after this SQL save fails.
    for target in results {
        target.status = TargetStatus::Unknown;
        target.message = detail.to_string();
    }
}
