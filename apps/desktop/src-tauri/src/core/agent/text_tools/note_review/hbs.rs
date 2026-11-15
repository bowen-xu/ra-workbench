//! Migration bridge: the live legacy proposal and HBS share S/B, while frozen
//! conversation snapshots and metadata decisions keep their original identity.
use super::*;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
#[cfg(test)]
#[path = "hbs_tests.rs"]
mod tests;

fn active(db: &Connection, page: &str) -> Result<Vec<(String, String, PendingNote)>, String> {
    let mut statement = db.prepare("select conversation_id,proposal_id,candidate_json from agent_note_patch_candidates where page_id=?1 and (state='pending' or (state='accepted' and persistence_version=2)) order by rowid").map_err(|e| e.to_string())?;
    statement
        .query_map([page], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .map(|row| {
            let (conversation, proposal, raw) = row.map_err(|e| e.to_string())?;
            let note = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            Ok((conversation, proposal, note))
        })
        .collect()
}

pub(crate) fn legacy_hbs_baseline(
    db: &Connection,
    page: &str,
) -> Result<Option<NoteDocument>, String> {
    let mut baseline = None;
    for (_, _, note) in active(db, page)? {
        if resolved(&note) {
            continue;
        }
        let mut candidate = baseline_document(&note);
        if candidate.title.is_none() {
            candidate.title = baseline_metadata(&note).title;
        }
        if baseline.as_ref().is_some_and(|prior: &NoteDocument| {
            !chunks::equal(prior, &candidate) || prior.title != candidate.title
        }) {
            return Err(
                "MODEL_LEGACY_REVIEW_CONFLICT：同一页面有不同的待审基线，请先核对原提案".into(),
            );
        }
        baseline = Some(candidate);
    }
    Ok(baseline)
}

/// Old deferred-save proposals already have durable candidates. Import these
/// into recovery without pretending that the candidate was saved or accepted.
pub(crate) fn legacy_hbs_checkpoint(db: &Connection, page: &str) -> Result<Option<Value>, String> {
    fn document(page: &StoredNote) -> NoteDocument {
        let mut document = page.document.clone();
        if document.title.is_none() {
            document.title = Some(page.title.clone());
        }
        document
    }
    let mut checkpoint = None;
    for (conversation, proposal, note) in active(db, page)? {
        if resolved(&note) {
            continue;
        }
        let Some(candidate) = note.candidate.as_ref() else {
            continue;
        };
        let saved = note.saved.as_ref().or(note.baseline.as_ref());
        let Some(saved) = saved else { continue };
        let mut baseline = baseline_document(&note);
        if baseline.title.is_none() {
            baseline.title = baseline_metadata(&note).title;
        }
        let frozen: Option<String> = db.query_row("select notes_json from agent_note_patch_snapshots where conversation_id=?1 and proposal_id=?2 order by generation desc limit 1", params![conversation,proposal], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
        let frozen = frozen
            .map(|raw| serde_json::from_str::<Vec<PendingNote>>(&raw).map_err(|e| e.to_string()))
            .transpose()?;
        let review_ancestor = frozen
            .as_ref()
            .and_then(|notes| notes.iter().find(|entry| id(entry) == page))
            .and_then(|note| note.candidate.as_ref())
            .map(document)
            .unwrap_or_else(|| {
                if note.manual_edited {
                    baseline.clone()
                } else {
                    document(candidate)
                }
            });
        let value = json!({"format":1,"kind":"note","observed":{"version":saved.revision,"saved":document(saved),"baseline":baseline},"content":document(candidate),"ancestor":document(saved),"reviewAncestor":review_ancestor,"baselineAncestor":baseline,"ordinary":[]});
        if checkpoint.as_ref().is_some_and(|prior| prior != &value) {
            return Err(
                "MODEL_LEGACY_REVIEW_CONFLICT：同一页面有不同的持久候选，请先核对原提案".into(),
            );
        }
        checkpoint = Some(value);
    }
    Ok(checkpoint)
}

pub(crate) fn synchronize_hbs_commit(
    tx: &rusqlite::Transaction<'_>,
    before: &StoredNote,
    after: &StoredNote,
    baseline: &NoteDocument,
) -> Result<(), String> {
    for (conversation, proposal, mut note) in active(tx, &before.page_id)? {
        let acknowledged = note.saved.as_ref().or(note.baseline.as_ref());
        if serde_json::to_value(acknowledged).map_err(|e| e.to_string())?
            != serde_json::to_value(Some(before)).map_err(|e| e.to_string())?
        {
            continue;
        }
        if let Some(candidate) = &mut note.candidate {
            candidate.document = after.document.clone();
            if before.title != after.title {
                candidate.title = after.title.clone();
            }
            candidate.revision = after.revision;
        }
        note.saved = Some(after.clone());
        note.review_document = Some(baseline.clone());
        if let Some(title) = &baseline.title {
            let mut metadata = baseline_metadata(&note);
            metadata.title = crate::notes::validate_title(title);
            note.review_metadata = Some(metadata);
        }
        note.manual_edited = true;
        let state = if resolved(&note) {
            "accepted"
        } else {
            "pending"
        };
        tx.execute("update agent_note_patch_candidates set candidate_json=?4,state=?5,persistence_version=2 where conversation_id=?1 and page_id=?2 and proposal_id=?3",params![conversation,before.page_id,proposal,serde_json::to_string(&note).map_err(|e| e.to_string())?,state]).map_err(|e| e.to_string())?;
    }
    Ok(())
}
