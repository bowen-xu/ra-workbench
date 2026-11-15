//! Explicit user recovery of a stale proposal mirror. H/B/S are resolved by
//! the existing HBS session first; this transaction never writes a document,
//! changes lifecycle, or accepts another page's proposal.
use super::*;
use rusqlite::OptionalExtension;

pub(super) fn resolve_in(store: &AppStore, request: Request) -> Result<Receipt, String> {
    let unknown = |error: String| {
        format!(
            "MODEL_NOTE_RECOVERY_UNKNOWN: completion is unknown; request={}; {error}",
            request.request_id
        )
    };
    let mut db = store
        .connection()
        .map_err(|error| unknown(error.to_string()))?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| unknown(error.to_string()))?;
    if let Some(receipt) = history::existing(&tx, &request).map_err(&unknown)? {
        return Ok(receipt);
    }
    decisions::active(&tx, request.conversation_id)?;
    let row: Option<(i64,String)> = tx.query_row("select generation,candidate_json from agent_note_patch_candidates where conversation_id=?1 and proposal_id=?2 and page_id=?3 and state in ('pending','accepted')",params![request.conversation_id.to_string(),request.proposal_id,request.page_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|error|error.to_string())?;
    let (generation, raw) = row.ok_or_else(|| {
        format!(
            "MODEL_NOTE_RECOVERY_STALE：提案已变化；page={} request={}",
            request.page_id, request.request_id
        )
    })?;
    if generation != request.generation {
        return Err(format!(
            "MODEL_NOTE_RECOVERY_STALE：提案版本已变化；page={} request={}",
            request.page_id, request.request_id
        ));
    }
    let before: PendingNote = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let current = super::super::note::load(&tx, &request.page_id)?;
    let authority = crate::notes::hbs::read(&tx, &request.page_id)?;
    if Some(current.revision) != request.expected_revision {
        return Err(format!(
            "MODEL_NOTE_RECOVERY_STALE：页面已变化；page={} currentRevision={} request={}",
            request.page_id, current.revision, request.request_id
        ));
    }
    // The supplied document is proof of the user's acknowledged choice, never
    // a second body writer. Inactive pages can settle a mirror only if their
    // unchanged saved content was already confirmed; no restore is implied.
    if serde_json::to_value(
        request
            .document
            .as_ref()
            .ok_or("MODEL_INVALID_REQUEST：缺少当前正文")?,
    )
    .map_err(|error| error.to_string())?
        != authority["saved"]
        || authority["saved"] != authority["baseline"]
    {
        return Err(format!(
            "MODEL_NOTE_RECOVERY_SAVE_REQUIRED：请先通过正文 HBS 流程保留当前内容并确认保存；page={} request={}",
            request.page_id, request.request_id
        ));
    }
    let mut after = before.clone();
    after.candidate = Some(current.clone());
    after.saved = Some(current);
    after.review_document = Some(
        serde_json::from_value(authority["baseline"].clone()).map_err(|error| error.to_string())?,
    );
    after.review_metadata = Some(metadata(after.candidate.as_ref()));
    after.reviewed_metadata = true;
    after.manual_edited = true;
    tx.execute("update agent_note_patch_candidates set candidate_json=?4,state='accepted',persistence_version=2 where conversation_id=?1 and proposal_id=?2 and page_id=?3",params![request.conversation_id.to_string(),request.proposal_id,request.page_id,serde_json::to_string(&after).map_err(|error|error.to_string())?]).map_err(|error|error.to_string())?;
    let receipt = Receipt {
        request_id: request.request_id,
        proposal_id: request.proposal_id.clone(),
        action: request.action.clone(),
        generation,
        review: None,
        pages: vec![],
    };
    // Keep the superseded candidate and exact recovery receipt in the original
    // review history. No private queue, destructive cleanup or new undo stack.
    history::persist(&tx, &request, &receipt, &[before], &[after])?;
    tx.commit().map_err(|error| unknown(error.to_string()))?;
    Ok(receipt)
}
