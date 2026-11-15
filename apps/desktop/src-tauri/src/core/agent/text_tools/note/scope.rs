//! Note hosts retain their selection ceiling behind the virtual file view.
use super::*;
pub(in crate::agent::text_tools) fn creation_source(
    backend: &NoteBackend,
    db: &Connection,
    note: &PendingNote,
) -> Result<String, String> {
    if let Some(parent) = note
        .candidate
        .as_ref()
        .and_then(|page| page.parent_id.clone())
    {
        return Ok(parent);
    }
    if note.original_path.starts_with("source://") {
        return Ok(backend.location(db, &note.original_path, None)?.1);
    }
    address(&note.original_path)?
        .0
        .ok_or("new note has no authorized creation source".into())
}
pub(super) fn authorized_creation(
    backend: &NoteBackend,
    db: &Connection,
    page: &str,
) -> Result<bool, String> {
    let Some(created) = pending(db, backend.record.conversation_id)?
        .into_iter()
        .find(|note| {
            note.baseline.is_none()
                && note
                    .candidate
                    .as_ref()
                    .is_some_and(|candidate| candidate.page_id == page)
        })
    else {
        return Ok(false);
    };
    let source = creation_source(backend, db, &created)?;
    Ok(selected(&backend.record, &source).is_none()
        && (authorized(db, &backend.record, &source, true)?
            || crate::agent::context_session::resource_tools::note_access(
                &backend.store,
                &backend.record,
                db,
                &source,
                true,
                backend.scope.clone(),
            )?))
}
pub(super) fn selected(record: &SubmissionRecord, page: &str) -> Option<String> {
    record.input.context.iter().find_map(|source| {
        let anchor: Value = serde_json::from_str(source.anchor.as_deref()?).ok()?;
        if anchor["pageId"] != page || !anchor["textViewVersion"].is_number() {
            return None;
        }
        anchor["blockId"].as_str().map(str::to_owned)
    })
}
pub(super) fn authorized(
    db: &Connection,
    record: &SubmissionRecord,
    page: &str,
    write: bool,
) -> Result<bool, String> {
    for source in &record.input.context {
        let Some(anchor) = source
            .anchor
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
        else {
            continue;
        };
        if anchor["pageId"] != page
            || !anchor["textViewVersion"].is_number()
            || (write && anchor["contextAccess"] == "readOnly")
        {
            continue;
        }
        let resource = if anchor["readingNoteId"].is_string() {
            crate::agent::note_submission::source::Resource::Reading(
                Uuid::parse_str(&source.resource_id).map_err(|_| "invalid note binding")?,
            )
        } else {
            crate::agent::note_submission::source::Resource::Page(
                Uuid::parse_str(page).map_err(|_| "invalid note binding")?,
            )
        };
        return Ok(resource.check_binding(db, record.conversation_id).is_ok());
    }
    Ok(false)
}
pub(super) fn restorable(
    db: &Connection,
    record: &SubmissionRecord,
    page: &str,
) -> Result<bool, String> {
    for source in &record.input.context {
        let Some(anchor) = source
            .anchor
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
        else {
            continue;
        };
        if anchor["pageId"] != page
            || !anchor["textViewVersion"].is_number()
            || anchor["contextAccess"] == "readOnly"
        {
            continue;
        }
        let (kind, id) = if anchor["readingNoteId"].is_string() {
            ("readingNote", source.resource_id.as_str())
        } else {
            ("note", page)
        };
        let allowed: bool = db.query_row("select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id where b.conversation_id=?1 and b.resource_kind=?2 and b.resource_id=?3 and c.archived=0)", params![record.conversation_id.to_string(),kind,id], |row| row.get(0)).map_err(|error| error.to_string())?;
        if allowed {
            return Ok(true);
        }
    }
    Ok(false)
}
fn find<'a>(blocks: &'a [NoteDocumentBlock], id: &str) -> Option<&'a NoteDocumentBlock> {
    blocks.iter().find_map(|block| {
        if block.id == id {
            Some(block)
        } else {
            find(&block.children, id)
        }
    })
}
pub(super) fn view(record: &SubmissionRecord, note: &StoredNote) -> Result<NoteDocument, String> {
    if let Some(id) = selected(record, &note.page_id) {
        return Ok(NoteDocument {
            blocks: vec![
                find(&note.document.blocks, &id)
                    .ok_or("selected block changed; refresh the context")?
                    .clone(),
            ],
            ..note.document.clone()
        });
    }
    Ok(note.document.clone())
}
pub(super) fn merge(
    record: &SubmissionRecord,
    note: &StoredNote,
    title: &str,
    mut edited: NoteDocument,
) -> Result<NoteDocument, String> {
    let Some(id) = selected(record, &note.page_id) else {
        return Ok(edited);
    };
    if title != note.title {
        return Err("a selected-block view cannot change the page title".into());
    }
    fn replace(
        blocks: &mut Vec<NoteDocumentBlock>,
        id: &str,
        next: &mut Vec<NoteDocumentBlock>,
    ) -> bool {
        if let Some(index) = blocks.iter().position(|block| block.id == id) {
            blocks.splice(index..=index, next.drain(..));
            return true;
        }
        blocks
            .iter_mut()
            .any(|block| replace(&mut block.children, id, next))
    }
    let mut document = note.document.clone();
    if !replace(&mut document.blocks, &id, &mut edited.blocks) {
        return Err("selected block changed; refresh context".into());
    }
    Ok(document)
}
pub(in crate::agent::text_tools) fn retains_scope(
    original: &SubmissionRecord,
    latest: &SubmissionRecord,
    page: &str,
) -> Result<(), String> {
    if let Some(selection) = selected(latest, page)
        && selected(original, page).as_deref() != Some(selection.as_str())
    {
        return Err("current selected-block authorization differs; regenerate the proposal".into());
    }
    Ok(())
}
