//! Once an effect starts, a missing receipt cannot become ordinary retry feedback.
use super::*;
use crate::{
    data_interface::{DataError, DatabaseGuard},
    storage::StorageError,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CallSave {
    Progress,
    Terminal,
    Recovery,
}
impl CallSave {
    fn phase(self) -> &'static str {
        match self {
            Self::Progress => "call-progress",
            Self::Terminal => "call-terminal",
            Self::Recovery => "call-recovery",
        }
    }
}
pub(super) enum SaveFailure {
    Definite(String),
    Unknown(Value),
}
impl SaveFailure {
    pub(super) fn into_result(self) -> Result<Value, String> {
        match self {
            Self::Definite(error) => Err(error),
            Self::Unknown(value) => Ok(value),
        }
    }
}
fn identity(conversation: Uuid, request: Option<Uuid>, key: &str) -> Value {
    let request = request.or_else(|| {
        key.split(':')
            .next()
            .and_then(|value| Uuid::parse_str(value).ok())
    });
    json!({"code":"TEXT_PATCH_JOURNAL_UNCONFIRMED","conversationId":conversation,"requestId":request,
        "callId":null,"callRequestId":null,"targetRequestId":null,"journalRequestId":key})
}
pub(super) fn sql_diagnostic(
    conversation: Uuid,
    request: Option<Uuid>,
    key: &str,
    phase: &'static str,
    error: &rusqlite::Error,
) -> Value {
    let mut value = identity(conversation, request, key);
    let (code, extended) = match error {
        rusqlite::Error::SqliteFailure(error, _) => {
            (Some(format!("{:?}", error.code)), Some(error.extended_code))
        }
        _ => (None, None),
    };
    value["phase"] = json!(phase);
    value["sqliteCode"] = json!(code);
    value["sqliteExtendedCode"] = json!(extended);
    value["safeCause"] = json!(if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
        "journal_receipt_missing"
    } else {
        "journal_sql_failed"
    });
    value
}
pub(super) fn prewrite_diagnostic(
    conversation: Uuid,
    key: &str,
    phase: &'static str,
    error: &rusqlite::Error,
) -> Value {
    let mut detail = sql_diagnostic(conversation, None, key, phase, error);
    detail["code"] = json!("TEXT_PATCH_JOURNAL_UNAVAILABLE");
    detail
}
pub(super) fn call_identity(value: &mut Value, record: &SubmissionRecord, key: &str) {
    value["callRequestId"] = json!(key);
    value["callId"] = json!(key.strip_prefix(&format!("{}:", record.request_id)));
}
pub(super) fn target_identity(value: &mut Value, record: &SubmissionRecord, key: &str) {
    value["targetRequestId"] = json!(key);
    if let Some(call) = key.strip_suffix(":notes") {
        call_identity(value, record, call);
    } else if let Some((call, index)) = key.rsplit_once(":file:")
        && index.parse::<usize>().is_ok()
    {
        call_identity(value, record, call);
    }
}
pub(super) fn call_diagnostic(
    record: &SubmissionRecord,
    key: &str,
    phase: &'static str,
    error: &rusqlite::Error,
) -> Value {
    let mut value = sql_diagnostic(
        record.conversation_id,
        Some(record.request_id),
        key,
        phase,
        error,
    );
    call_identity(&mut value, record, key);
    value
}
pub(super) fn target_diagnostic(
    record: &SubmissionRecord,
    key: &str,
    phase: &'static str,
    error: &rusqlite::Error,
) -> Value {
    let mut value = sql_diagnostic(
        record.conversation_id,
        Some(record.request_id),
        key,
        phase,
        error,
    );
    target_identity(&mut value, record, key);
    value
}
pub(super) fn unavailable_receipt(
    record: &SubmissionRecord,
    key: &str,
    error: Option<&str>,
) -> Value {
    if let Some(error) = error.filter(|error| error.len() <= 8 * 1024)
        && let Ok(detail) = serde_json::from_str::<Value>(error)
        && let Some(detail) = patch_diagnostic::query_detail(record, key, &detail)
    {
        return detail;
    }
    let mut value = identity(record.conversation_id, Some(record.request_id), key);
    value["targetRequestId"] = json!(key);
    value["phase"] = json!("target-query");
    value["sqliteCode"] = Value::Null;
    value["sqliteExtendedCode"] = Value::Null;
    value["safeCause"] = json!("target_receipt_unavailable");
    target_identity(&mut value, record, key);
    // Only our owned, identity-matched backend diagnostic can retain numeric
    // driver evidence. Arbitrary backend messages never become display text.
    if let Some(Ok(detail)) = error.map(serde_json::from_str::<Value>)
        && detail["code"] == "TEXT_PATCH_JOURNAL_UNCONFIRMED"
        && detail["conversationId"] == json!(record.conversation_id)
        && detail["requestId"] == json!(record.request_id)
        && detail["journalRequestId"] == json!(key)
        && let Some(code) = detail["sqliteExtendedCode"]
            .as_i64()
            .and_then(|code| i32::try_from(code).ok())
    {
        value["sqliteCode"] = json!(format!("{:?}", rusqlite::ffi::Error::new(code).code));
        value["sqliteExtendedCode"] = json!(code);
    }
    value
}
pub(super) fn query_owner_diagnostic(
    record: &SubmissionRecord,
    key: &str,
    error: &StorageError,
) -> Value {
    if let StorageError::Database(cause) | StorageError::DataInterface(DataError::Sql(cause)) =
        error
    {
        target_diagnostic(record, key, "target-query", cause)
    } else {
        unavailable_receipt(record, key, None)
    }
}
pub(super) fn owner_diagnostic(
    record: &SubmissionRecord,
    key: &str,
    phase: &'static str,
    error: &StorageError,
) -> Value {
    if let StorageError::Database(cause) | StorageError::DataInterface(DataError::Sql(cause)) =
        error
    {
        return call_diagnostic(record, key, phase, cause);
    }
    let mut value = identity(record.conversation_id, Some(record.request_id), key);
    value["phase"] = json!(phase);
    value["sqliteCode"] = Value::Null;
    value["sqliteExtendedCode"] = Value::Null;
    value["safeCause"] = json!("journal_owner_unavailable");
    call_identity(&mut value, record, key);
    value
}
pub(super) fn unknown_changes(changes: &[TextChange], detail: &Value) -> Vec<TargetResult> {
    changes
        .iter()
        .map(|change| TargetResult {
            path: change.path.clone(),
            name: None,
            status: TargetStatus::Unknown,
            message: detail.to_string(),
            change_id: None,
            generation: None,
        })
        .collect()
}
pub(super) fn unknown(mut value: Value, detail: Value) -> Value {
    value["status"] = json!("unknown");
    if value.get("storageFailure").is_none() {
        value["storageFailure"] = detail;
    } else if value["storageFailure"] != detail {
        let details = value.as_object_mut().and_then(|value| {
            value
                .entry("additionalStorageFailures")
                .or_insert_with(|| json!([]))
                .as_array_mut()
        });
        if let Some(details) = details
            && !details.contains(&detail)
        {
            details.push(detail);
        }
    }
    value
}
pub(super) fn uncertain_targets(results: &mut [TargetResult], detail: &Value) {
    for result in results {
        if matches!(
            result.status,
            TargetStatus::Applied | TargetStatus::Proposed | TargetStatus::Unknown
        ) {
            result.status = TargetStatus::Unknown;
            result.message = detail.to_string();
        }
    }
}
fn failure(value: &Value, mut detail: Value, uncertain: bool) -> SaveFailure {
    if uncertain {
        report(&detail);
        SaveFailure::Unknown(unknown(value.clone(), detail))
    } else {
        detail["code"] = json!("TEXT_PATCH_JOURNAL_UNAVAILABLE");
        SaveFailure::Definite(detail.to_string())
    }
}
pub(super) fn report(detail: &Value) {
    // A failed SQL write cannot durably save its own diagnostic. Retain only
    // this closed safe DTO in the native log; never print the patch/result.
    match research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(detail.clone()) {
        Ok(detail) => eprintln!("Agent patch receipt requires recovery: {detail}"),
        Err(_) => eprintln!("Agent patch receipt requires recovery: safe diagnostic unavailable"),
    }
}
impl TextTools {
    pub(super) fn save_patch_call<'a>(
        &'a self,
        key: &str,
        value: &Value,
        results: &[TargetResult],
        stage: CallSave,
    ) -> Result<DatabaseGuard<'a>, SaveFailure> {
        // A later outer save/owner failure must not replace the original
        // target's known phase and driver cause with a generic lock failure.
        let original_detail = results
            .iter()
            .filter(|result| result.status == TargetStatus::Unknown)
            .find_map(|result| {
                if result.message.len() > 8 * 1024 {
                    return None;
                }
                let detail: Value = serde_json::from_str(&result.message).ok()?;
                let target = detail["targetRequestId"].as_str()?;
                if target != format!("{key}:notes")
                    && target
                        .strip_prefix(key)
                        .and_then(|suffix| suffix.strip_prefix(":file:"))
                        .is_none_or(|index| index.parse::<usize>().is_err())
                {
                    return None;
                }
                patch_diagnostic::query_detail(&self.record, target, &detail)
            });
        let preserved = original_detail.map(|detail| unknown(value.clone(), detail));
        let value = preserved.as_ref().unwrap_or(value);
        let uncertain = stage == CallSave::Recovery
            || results.iter().any(|result| {
                matches!(
                    result.status,
                    TargetStatus::Applied | TargetStatus::Proposed | TargetStatus::Unknown
                )
            });
        let mut db = self.store.connection().map_err(|error| {
            failure(
                value,
                owner_diagnostic(&self.record, key, stage.phase(), &error),
                uncertain,
            )
        })?;
        if !db.is_autocommit() {
            db.require_recovery();
            let mut detail = unavailable_receipt(&self.record, key, None);
            call_identity(&mut detail, &self.record, key);
            detail["phase"] = json!(stage.phase());
            detail["safeCause"] = json!("note_transaction_unconfirmed");
            return Err(failure(value, detail, true));
        }
        let complete = stage != CallSave::Progress
            && value["status"] != "unknown"
            && !results
                .iter()
                .any(|result| result.status == TargetStatus::Unknown);
        journal::record_sql(&db, self.record.conversation_id, key, value, complete).map_err(
            |error| {
                failure(
                    value,
                    call_diagnostic(&self.record, key, stage.phase(), &error),
                    uncertain,
                )
            },
        )?;
        Ok(db)
    }
}
