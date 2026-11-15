//! Formal D and review B commit independently. The original snapshot is kept
//! for whole-operation Undo; `saved` is the evolving CAS credential S.
use super::*;
use crate::notes::text_change_set::{PageChange, apply};

pub(in crate::agent::text_tools) fn checkpoint(
    tx: &rusqlite::Transaction<'_>,
    group: &mut [PendingNote],
) -> Result<Vec<StoredNote>, String> {
    for note in group.iter_mut() {
        note.review_document = Some(baseline_document(note));
        note.review_metadata = Some(baseline_metadata(note));
    }
    let changes = group
        .iter()
        .filter_map(|note| {
            let before = note.saved.as_ref().or(note.baseline.as_ref());
            let changed = match (before, note.candidate.as_ref()) {
                (Some(before), Some(after)) => {
                    !metadata_equal(Some(before), Some(after))
                        || !chunks::equal(&before.document, &after.document)
                }
                (Some(before), None) => before.status != "trashed",
                (None, Some(_)) => true,
                (None, None) => false,
            };
            changed.then(|| PageChange {
                before: before.cloned(),
                after: note.candidate.clone(),
            })
        })
        .collect::<Vec<_>>();
    let pages = apply(tx, &changes).map_err(|error| error.to_string())?;
    for note in group {
        let saved = note::load(tx, id(note))?;
        if let Some(candidate) = &mut note.candidate {
            // The repository may normalize related page ordering/labels. Keep
            // the acknowledged native projection as D, without advancing B.
            *candidate = saved.clone();
        }
        note.saved = Some(saved);
        // Publication establishes resource metadata, not acceptance of the
        // generated document. Keep review_document's independent B intact.
        note.review_metadata = Some(metadata(note.candidate.as_ref()));
        note.reviewed_metadata = true;
        crate::notes::hbs::synchronize_baseline_if_initialized(
            tx,
            id(note),
            &baseline_document(note),
        )?;
    }
    Ok(pages)
}

pub(in crate::agent::text_tools) fn matches(db: &Connection, note: &PendingNote) -> bool {
    note.saved
        .as_ref()
        .or(note.baseline.as_ref())
        .is_none_or(|saved| {
            note::load(db, &saved.page_id).ok().is_some_and(|current| {
                serde_json::to_value(current).ok() == serde_json::to_value(saved).ok()
            })
        })
}

/// A user metadata write advances S in the same transaction. Only the exact
/// preceding saved snapshot may be adopted; unrelated external writes remain conflicts.
pub(crate) fn synchronize_metadata(
    tx: &rusqlite::Transaction<'_>,
    before: &StoredNote,
    after: &StoredNote,
) -> Result<(), String> {
    let mut statement = tx.prepare("select conversation_id,proposal_id,candidate_json from agent_note_patch_candidates where page_id=?1 and persistence_version=2 and state in ('pending','accepted')").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([&before.page_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    for (conversation, proposal, encoded) in rows {
        let mut note: PendingNote = serde_json::from_str(&encoded).map_err(|e| e.to_string())?;
        if serde_json::to_value(&note.saved).map_err(|e| e.to_string())?
            != serde_json::to_value(Some(before)).map_err(|e| e.to_string())?
        {
            continue;
        }
        let mut baseline = baseline_metadata(&note);
        if let Some(candidate) = &mut note.candidate {
            if before.title != after.title {
                candidate.title.clone_from(&after.title);
                baseline.title = Some(after.title.clone());
                note.manual_edited = true;
            }
            if before.parent_id != after.parent_id {
                candidate.parent_id.clone_from(&after.parent_id);
                baseline.parent_id.clone_from(&after.parent_id);
                note.manual_edited = true;
            }
            if before.position != after.position {
                candidate.position = after.position;
                baseline.position = after.position;
                note.manual_edited = true;
            }
            if before.status != after.status {
                candidate.status.clone_from(&after.status);
                baseline.status = Some(after.status.clone());
                note.manual_edited = true;
            }
            candidate.revision = after.revision;
        }
        note.review_metadata = Some(baseline);
        note.saved = Some(after.clone());
        tx.execute("update agent_note_patch_candidates set candidate_json=?4 where conversation_id=?1 and page_id=?2 and proposal_id=?3", params![conversation, before.page_id, proposal, serde_json::to_string(&note).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    }
    Ok(())
}
