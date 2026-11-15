//! Trusted rich-review history and exact-request receipts, in the mutation transaction.
use super::*;
use rusqlite::OptionalExtension;
use std::collections::BTreeSet;
type Source = (Vec<PendingNote>, Vec<PendingNote>, Vec<StoredNote>);

fn fingerprint(request: &Request) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(request).map_err(|e| e.to_string())?)
    ))
}
pub(super) fn existing(db: &Connection, request: &Request) -> Result<Option<Receipt>, String> {
    decisions::active(db, request.conversation_id)?;
    let row: Option<(String, String)> = db
        .query_row(
            "select input_sha256,result_json from agent_note_review_receipts where request_id=?1",
            [request.request_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    row.map(|(hash, json)| {
        if hash != fingerprint(request)? {
            return Err("request identity belongs to another rich review operation".into());
        }
        serde_json::from_str(&json).map_err(|e| e.to_string())
    })
    .transpose()
}
pub(super) fn persist(
    db: &Connection,
    request: &Request,
    receipt: &Receipt,
    before: &[PendingNote],
    after: &[PendingNote],
) -> Result<(), String> {
    let formal = after
        .iter()
        .filter_map(|note| match note::load(db, id(note)) {
            Ok(page) => Some(Ok(page)),
            Err(_) if note.baseline.is_none() => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    db.execute("insert into agent_note_review_receipts(request_id,conversation_id,proposal_id,input_sha256,result_json,before_json,after_json,formal_json) values(?1,?2,?3,?4,?5,?6,?7,?8)", params![request.request_id.to_string(), request.conversation_id.to_string(), request.proposal_id, fingerprint(request)?, serde_json::to_string(receipt).map_err(|e|e.to_string())?, serde_json::to_string(before).map_err(|e|e.to_string())?, serde_json::to_string(after).map_err(|e|e.to_string())?, serde_json::to_string(&formal).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    Ok(())
}
fn source(db: &Connection, request: &Request) -> Result<Source, String> {
    let history = request
        .history_request_id
        .ok_or("restore requires an original history request")?;
    let (before, after, formal): (String, String, String) = db.query_row("select before_json,after_json,formal_json from agent_note_review_receipts where request_id=?1 and conversation_id=?2 and proposal_id=?3", params![history.to_string(), request.conversation_id.to_string(), request.proposal_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|_| "trusted review history is unavailable")?;
    Ok((
        serde_json::from_str(&before).map_err(|e| e.to_string())?,
        serde_json::from_str(&after).map_err(|e| e.to_string())?,
        serde_json::from_str(&formal).map_err(|e| e.to_string())?,
    ))
}
pub(super) fn targets(db: &Connection, request: &Request) -> Result<BTreeSet<String>, String> {
    let (before, after, _) = source(db, request)?;
    Ok(before
        .iter()
        .chain(&after)
        .map(|note| id(note).to_owned())
        .collect())
}
fn review_equal(a: &[PendingNote], b: &[PendingNote]) -> bool {
    a.len() == b.len()
        && a.iter().all(|a| {
            b.iter().find(|b| id(a) == id(b)).is_some_and(|b| {
                metadata_equal(a.candidate.as_ref(), b.candidate.as_ref())
                    && a.reviewed_metadata == b.reviewed_metadata
                    && baseline_metadata(a) == baseline_metadata(b)
                    && chunks::equal(&baseline_document(a), &baseline_document(b))
                    && match (&a.candidate, &b.candidate) {
                        (Some(a), Some(b)) => chunks::equal(&a.document, &b.document),
                        (None, None) => true,
                        _ => false,
                    }
            })
        })
}
pub(super) fn restore(
    db: &Connection,
    request: &Request,
) -> Result<(Vec<PendingNote>, Vec<PendingNote>), String> {
    let (before, after, formal) = source(db, request)?;
    let (expected, mut target) = match request.direction.as_deref() {
        Some("undo") => (&after, before.clone()),
        Some("redo") => (&before, after.clone()),
        _ => return Err("restore direction must be undo or redo".into()),
    };
    let mut query = db.prepare("select generation,candidate_json from agent_note_patch_candidates where conversation_id=?1 and proposal_id=?2 order by page_id").map_err(|e|e.to_string())?;
    let rows = query
        .query_map(
            params![request.conversation_id.to_string(), request.proposal_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty()
        || rows
            .iter()
            .any(|(generation, _)| *generation != request.generation)
    {
        return Err("review history is stale after a newer proposal".into());
    }
    let current = rows
        .into_iter()
        .map(|(_, json)| serde_json::from_str::<PendingNote>(&json).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if !current.iter().any(|note| id(note) == request.page_id) {
        return Err("page is outside this review history".into());
    }
    if !review_equal(&current, expected) {
        return Err("review history no longer matches the live document and baseline".into());
    }
    // Live checkpoints may have legitimately saved several later review/history
    // actions. Validate their current S, then compare the trusted D/B transition.
    // A formal write outside this owner changes S and still fails.
    let live = current.iter().any(|note| note.saved.is_some());
    if live
        && !current
            .iter()
            .all(|note| super::persistence::matches(db, note))
    {
        return Err("formal note changed after this review action; preserve current input".into());
    }
    for page in formal.iter().filter(|_| !live) {
        let current = note::load(db, &page.page_id)?;
        if serde_json::to_value(current).map_err(|e| e.to_string())?
            != serde_json::to_value(page).map_err(|e| e.to_string())?
        {
            return Err(
                "formal note changed after this review action; preserve current input".into(),
            );
        }
    }
    for note in &mut target {
        if live {
            note.saved = current
                .iter()
                .find(|page| id(page) == id(note))
                .and_then(|page| page.saved.clone());
            if let (Some(candidate), Some(saved)) = (&mut note.candidate, &note.saved) {
                candidate.revision = saved.revision;
            }
            continue;
        }
        let review_document = baseline_document(note);
        let review_metadata = baseline_metadata(note);
        if let Some(formal) = formal.iter().find(|page| page.page_id == id(note)) {
            // Keep-all history is B-only: reopening review never rewinds saved S.
            note.baseline = Some(formal.clone());
            note.review_document = Some(review_document);
            note.review_metadata = Some(review_metadata);
            if let Some(candidate) = &mut note.candidate {
                candidate.revision = formal.revision;
            }
        }
    }
    Ok((current, target))
}
