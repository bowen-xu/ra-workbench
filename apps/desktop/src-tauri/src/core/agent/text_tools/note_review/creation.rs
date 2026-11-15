//! Compatibility for an already-published creation's obsolete metadata button.
//! A definite no-op receipt retires its exact request; it never accepts content,
//! restores a cut page, or clears an unverified body operation.
use super::*;
use rusqlite::OptionalExtension;
pub(super) fn settle_published_creation(
    store: &AppStore,
    request: &Request,
) -> Result<Option<Receipt>, String> {
    if !matches!(request.action.as_str(), "keep" | "undo") || request.chunk_id.is_some() {
        return Ok(None);
    }
    let mut db = store.connection().map_err(|e| e.to_string())?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    if let Some(receipt) = history::existing(&tx, request)? {
        return Ok(Some(receipt));
    }
    decisions::active(&tx, request.conversation_id)?;
    let row:Option<(i64,String)>=tx.query_row("select generation,candidate_json from agent_note_patch_candidates where conversation_id=?1 and proposal_id=?2 and page_id=?3 and state in ('pending','accepted') and persistence_version=2",params![request.conversation_id.to_string(),request.proposal_id,request.page_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|e|e.to_string())?;
    let Some((generation, raw)) = row else {
        return Ok(None);
    };
    let mut note: PendingNote = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if note.baseline.is_some() || note.saved.is_none() {
        return Ok(None);
    }
    let current = super::super::note::load(&tx, &request.page_id)?;
    let authority = crate::notes::hbs::read(&tx, &request.page_id)?;
    // Compare complete canonical documents, including the title. The resource
    // may now be in trash after a cut; that ordinary lifecycle fact stays put.
    let canonical = serde_json::to_value(&current.document).map_err(|e| e.to_string())?;
    if authority["saved"] != authority["baseline"]
        || note
            .candidate
            .as_ref()
            .is_none_or(|page| serde_json::to_value(&page.document).ok() != Some(canonical.clone()))
    {
        return Ok(None);
    }
    note.candidate = Some(current.clone());
    note.saved = Some(current);
    note.review_document =
        Some(serde_json::from_value(authority["baseline"].clone()).map_err(|e| e.to_string())?);
    note.review_metadata = Some(metadata(note.candidate.as_ref()));
    note.reviewed_metadata = true;
    tx.execute("update agent_note_patch_candidates set candidate_json=?4,state='accepted' where conversation_id=?1 and proposal_id=?2 and page_id=?3",params![request.conversation_id.to_string(),request.proposal_id,request.page_id,serde_json::to_string(&note).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    let receipt = Receipt {
        request_id: request.request_id,
        proposal_id: request.proposal_id.clone(),
        action: request.action.clone(),
        generation,
        review: None,
        pages: vec![],
    };
    // No history delta: accepting a published identity was never a body edit.
    history::persist(
        &tx,
        request,
        &receipt,
        std::slice::from_ref(&note),
        std::slice::from_ref(&note),
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(receipt))
}
