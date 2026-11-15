use super::*;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn remember(
    db: &Connection,
    record: &SubmissionRecord,
    snapshot: &TextSnapshot,
) -> Result<(), String> {
    let same: bool = db.query_row("select exists(select 1 from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2 and path=?3 and version=?4)", params![record.conversation_id.to_string(),record.request_id.to_string(),snapshot.path,snapshot.version], |row| row.get(0)).map_err(|error| error.to_string())?;
    if same {
        return Ok(());
    }
    db.execute("insert into agent_text_read_scopes(conversation_id,scope_request_id,path,version,snapshot_json) values(?1,?2,?3,?4,?5) on conflict(conversation_id,scope_request_id,path) do update set version=excluded.version,snapshot_json=excluded.snapshot_json",params![record.conversation_id.to_string(),record.request_id.to_string(),snapshot.path,snapshot.version,serde_json::to_string(snapshot).map_err(|error|error.to_string())?]).map_err(|error|error.to_string())?;
    Ok(())
}
pub(super) enum BeginFailure {
    Lookup(rusqlite::Error),
    Insert(rusqlite::Error),
    Conflict,
    Corrupt,
}
impl BeginFailure {
    pub(super) fn uncertain(&self) -> bool {
        matches!(self, Self::Lookup(_) | Self::Corrupt)
    }
    pub(super) fn detail(&self, conversation: Uuid, request: Option<Uuid>, key: &str) -> Value {
        let request = request.or_else(|| {
            key.split(':')
                .next()
                .and_then(|value| Uuid::parse_str(value).ok())
        });
        match self {
            Self::Lookup(error) => patch_journal::sql_diagnostic(
                conversation,
                request,
                key,
                "journal-begin-read",
                error,
            ),
            Self::Insert(error) => {
                patch_journal::prewrite_diagnostic(conversation, key, "journal-begin-insert", error)
            }
            Self::Conflict | Self::Corrupt => {
                json!({"code":if self.uncertain(){"TEXT_PATCH_JOURNAL_UNCONFIRMED"}else{"TEXT_PATCH_IDENTITY_CONFLICT"},"conversationId":conversation,"requestId":request,"journalRequestId":key,"phase":"journal-begin-read","sqliteCode":null,"sqliteExtendedCode":null,"safeCause":if self.uncertain(){"journal_result_invalid"}else{"patch_identity_reused"}})
            }
        }
    }
    pub(super) fn call_message(self, record: &SubmissionRecord, key: &str) -> String {
        let mut detail = self.detail(record.conversation_id, Some(record.request_id), key);
        patch_journal::call_identity(&mut detail, record, key);
        detail.to_string()
    }
    pub(super) fn target_message(self, record: &SubmissionRecord, key: &str) -> String {
        let mut detail = self.detail(record.conversation_id, Some(record.request_id), key);
        patch_journal::target_identity(&mut detail, record, key);
        detail.to_string()
    }
}
pub(super) fn lookup_sql(
    db: &Connection,
    conversation: Uuid,
    request: &str,
    input: &str,
) -> Result<Option<Value>, BeginFailure> {
    let hash = format!("{:x}", Sha256::digest(input));
    let existing:Option<(String,String,String)>=db.query_row("select input_sha256,state,results_json from agent_text_patch_calls where conversation_id=?1 and request_id=?2",params![conversation.to_string(),request],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(BeginFailure::Lookup)?;
    if let Some((old, state, result)) = existing {
        if old != hash {
            return Err(BeginFailure::Conflict);
        }
        let mut result: Value = serde_json::from_str(&result).map_err(|_| BeginFailure::Corrupt)?;
        if !result.is_object() {
            return Err(BeginFailure::Corrupt);
        }
        if state != "complete" {
            result["status"] = json!("unknown");
            result["message"] = json!(
                "Original call did not finish; inspect each target. Do not replay completed or unknown writes."
            );
        }
        return Ok(Some(result));
    }
    Ok(None)
}
pub(super) fn insert_sql(
    db: &Connection,
    conversation: Uuid,
    request: &str,
    input: &str,
) -> Result<(), BeginFailure> {
    let hash = format!("{:x}", Sha256::digest(input));
    let inserted=db.execute("insert into agent_text_patch_calls(conversation_id,request_id,input_sha256,state,results_json) values(?1,?2,?3,'running','{\"status\":\"unknown\",\"targets\":[]}')",params![conversation.to_string(),request,hash]).map_err(BeginFailure::Insert)?;
    if inserted != 1 {
        return Err(BeginFailure::Insert(rusqlite::Error::QueryReturnedNoRows));
    }
    Ok(())
}
pub(super) fn begin_sql(
    db: &Connection,
    conversation: Uuid,
    request: &str,
    input: &str,
) -> Result<Option<Value>, BeginFailure> {
    let prior = lookup_sql(db, conversation, request, input)?;
    if prior.is_none() {
        insert_sql(db, conversation, request, input)?;
    }
    Ok(prior)
}
pub(super) fn record_sql(
    db: &Connection,
    conversation: Uuid,
    request: &str,
    result: &Value,
    complete: bool,
) -> Result<(), rusqlite::Error> {
    let updated=db.execute("update agent_text_patch_calls set results_json=?3,state=?4 where conversation_id=?1 and request_id=?2",params![conversation.to_string(),request,result.to_string(),if complete{"complete"}else{"running"}])?;
    if updated != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}
