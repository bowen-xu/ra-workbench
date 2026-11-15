//! Stored/candidate snapshots and their canonical registered source identities.
use super::*;
pub(crate) use crate::notes::text_change_set::PageState as StoredNote;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReviewMetadata {
    pub exists: bool,
    pub title: Option<String>,
    pub parent_id: Option<String>,
    pub position: i64,
    pub status: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PendingNote {
    pub path: String,
    pub original_path: String,
    pub scope_request_id: Uuid,
    pub baseline: Option<StoredNote>,
    pub candidate: Option<StoredNote>,
    /// Last acknowledged formal S. Absent on pre-checkpoint proposals, which
    /// retain their deferred-save recovery contract until a new delivery.
    #[serde(default)]
    pub saved: Option<StoredNote>,
    #[serde(default)]
    pub review_document: Option<NoteDocument>,
    #[serde(default)]
    pub review_metadata: Option<ReviewMetadata>,
    #[serde(default)]
    pub reviewed_metadata: bool,
    #[serde(default)]
    pub manual_edited: bool,
    #[serde(default)]
    pub imports: Vec<(String, String)>,
}
pub(crate) fn load(db: &Connection, page: &str) -> Result<StoredNote, String> {
    crate::notes::text_change_set::read(db, page).map_err(|error| error.to_string())
}
pub(crate) fn canonical(note: &StoredNote) -> String {
    text_patch::address::object(
        text_patch::address::SourceRoot::Notes,
        std::slice::from_ref(&note.page_id),
    )
}
fn decode_pending(raw: String) -> Result<PendingNote, String> {
    let mut note: PendingNote = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    if let Some(page) = note.candidate.as_ref().or(note.baseline.as_ref()) {
        note.path = canonical(page);
    }
    Ok(note)
}
pub(super) fn pending_alias_targets(
    db: &Connection,
    conversation: Uuid,
    path: &str,
) -> Result<Vec<String>, String> {
    let mut statement=db.prepare("select page_id from agent_note_patch_candidates where conversation_id=?1 and state='pending' and (json_extract(candidate_json,'$.path')=?2 or json_extract(candidate_json,'$.originalPath')=?2)").map_err(|error|error.to_string())?;
    statement
        .query_map(params![conversation.to_string(), path], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())
}
/// Preserve the still-pending members of an addressed atomic proposal. An
/// unrelated proposal is neither a write prerequisite nor an implicit reject.
pub(super) fn pending_for_targets(
    db: &Connection,
    conversation: Uuid,
    targets: &std::collections::BTreeSet<String>,
) -> Result<Vec<PendingNote>, String> {
    let mut statement = db.prepare("select candidate_json from agent_note_patch_candidates where conversation_id=?1 and state='pending' and proposal_id in (select proposal_id from agent_note_patch_candidates where conversation_id=?1 and state='pending' and page_id in(select value from json_each(?2))) order by page_id").map_err(|error|error.to_string())?;
    statement
        .query_map(
            params![
                conversation.to_string(),
                serde_json::to_string(targets).map_err(|error| error.to_string())?
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?
        .map(|row| decode_pending(row.map_err(|error| error.to_string())?))
        .collect()
}
pub(crate) fn pending(db: &Connection, conversation: Uuid) -> Result<Vec<PendingNote>, String> {
    let mut statement=db.prepare("select candidate_json from agent_note_patch_candidates where conversation_id=?1 and state='pending' order by page_id").map_err(|error|error.to_string())?;
    statement
        .query_map([conversation.to_string()], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .map(|row| decode_pending(row.map_err(|error| error.to_string())?))
        .collect()
}
