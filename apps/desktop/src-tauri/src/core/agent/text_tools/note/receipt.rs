//! Original note results are read before preparation; missing proof is not permission to retry.
use super::*;

pub(super) fn transaction_detail(
    record: &SubmissionRecord,
    key: &str,
    phase: &'static str,
    error: Option<&rusqlite::Error>,
) -> Value {
    let mut detail = match error {
        Some(error) => patch_journal::target_diagnostic(record, key, phase, error),
        None => patch_journal::unavailable_receipt(record, key, None),
    };
    detail["phase"] = json!(phase);
    if error.is_none() {
        detail["safeCause"] = json!("note_transaction_unconfirmed");
    }
    if let Some(call) = key.strip_suffix(":notes") {
        patch_journal::call_identity(&mut detail, record, call);
    }
    detail
}

pub(super) fn lookup(
    db: &Connection,
    record: &SubmissionRecord,
    key: &str,
    input: &str,
    changes: &[TextChange],
) -> Result<Option<Vec<TargetResult>>, String> {
    let unknown = |detail: Value| {
        patch_journal::report(&detail);
        Ok(Some(patch_journal::unknown_changes(changes, &detail)))
    };
    if !db.is_autocommit() {
        return unknown(transaction_detail(record, key, "note-begin", None));
    }
    let prior = match journal::lookup_sql(db, record.conversation_id, key, input) {
        Ok(None) => return Ok(None),
        Ok(Some(prior)) => prior,
        Err(error) if error.uncertain() => {
            let mut detail = error.detail(record.conversation_id, Some(record.request_id), key);
            patch_journal::target_identity(&mut detail, record, key);
            return unknown(detail);
        }
        Err(error) => return Err(error.target_message(record, key)),
    };
    if prior["status"] != "unknown"
        && let Ok(results) = decode(&prior)
        && results.len() == changes.len()
        && results
            .iter()
            .zip(changes)
            .all(|(result, change)| result.path == change.path)
    {
        return Ok(Some(results));
    }
    unknown(patch_journal::unavailable_receipt(record, key, None))
}

fn decode(value: &Value) -> Result<Vec<TargetResult>, serde_json::Error> {
    serde_json::from_value(value["targets"].clone())
}

pub(super) fn query(
    db: &Connection,
    record: &SubmissionRecord,
    key: &str,
) -> Result<Option<Vec<TargetResult>>, String> {
    if !db.is_autocommit() {
        return Err(transaction_detail(record, key, "note-begin", None).to_string());
    }
    let value: Option<(String, String)> = db.query_row(
        "select state,results_json from agent_text_patch_calls where conversation_id=?1 and request_id=?2",
        params![record.conversation_id.to_string(), key], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional().map_err(|error| patch_journal::target_diagnostic(record,key,"target-query",&error).to_string())?;
    let Some((state, value)) = value else {
        return Ok(None);
    };
    if state == "complete"
        && let Ok(value) = serde_json::from_str(&value)
        && let Ok(results) = decode(&value)
        && !results.is_empty()
    {
        return Ok(Some(results));
    }
    Err(patch_journal::unavailable_receipt(record, key, None).to_string())
}
