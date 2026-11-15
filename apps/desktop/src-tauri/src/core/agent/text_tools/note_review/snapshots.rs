//! Frozen tool-call content for conversation cards, independent from the live B/D.
use super::*;
use rusqlite::OptionalExtension;
pub(in crate::agent::text_tools) fn first_generation(
    db: &Connection,
    conversation: Uuid,
    proposal: &str,
) -> Result<Option<i64>, String> {
    db.query_row("select generation from agent_note_patch_snapshots where conversation_id=?1 and proposal_id=?2 order by generation limit 1", params![conversation.to_string(),proposal], |row|row.get(0)).optional().map_err(|error|error.to_string())
}
pub(in crate::agent::text_tools) fn for_turn(
    group: &[PendingNote],
    previous: &[PendingNote],
) -> Vec<PendingNote> {
    group
        .iter()
        .filter_map(|note| {
            let mut snapshot = note.clone();
            if let Some(prior) = previous.iter().find(|prior| id(prior) == id(note)) {
                snapshot.baseline = prior.candidate.clone();
            }
            snapshot.review_document = snapshot.baseline.as_ref().map(|p| p.document.clone());
            snapshot.review_metadata = Some(metadata(snapshot.baseline.as_ref()));
            if serde_json::to_value(&snapshot.baseline).ok()
                == serde_json::to_value(&snapshot.candidate).ok()
            {
                None
            } else {
                Some(snapshot)
            }
        })
        .collect()
}

pub(in crate::agent::text_tools) fn record(
    db: &Connection,
    conversation: Uuid,
    proposal: &str,
    generation: i64,
    notes: &[PendingNote],
) -> Result<(), String> {
    db.execute("insert into agent_note_patch_snapshots(conversation_id,proposal_id,generation,notes_json) values(?1,?2,?3,?4)", params![conversation.to_string(), proposal, generation, serde_json::to_string(notes).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    Ok(())
}
pub(in crate::agent::text_tools) fn read(
    db: &Connection,
    conversation: Uuid,
    proposal: &str,
    generation: i64,
) -> Result<Option<decisions::Review>, String> {
    decisions::active(db, conversation)?;
    let snapshot: Option<String> = db.query_row("select notes_json from agent_note_patch_snapshots where conversation_id=?1 and proposal_id=?2 and generation=?3", params![conversation.to_string(),proposal,generation], |r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    let notes: Vec<PendingNote> = serde_json::from_str(&snapshot).map_err(|e| e.to_string())?;
    let current = decisions::review_selected(db, conversation, Some(proposal), None)?;
    let current = current.filter(|review| review.proposal_id == proposal);
    let first_accept: Option<String> = db.query_row("select result_json from agent_note_patch_decisions where conversation_id=?1 and proposal_id=?2 and action='accept' order by rowid limit 1", params![conversation.to_string(), proposal], |row|row.get(0)).optional().map_err(|e|e.to_string())?;
    let first_accept = first_accept
        .map(|value| serde_json::from_str::<decisions::Receipt>(&value).map_err(|e| e.to_string()))
        .transpose()?;
    let (state, baseline_matches, operation_generation, whole_undo_available) = if let Some(
        accepted,
    ) = first_accept
    {
        let undone: bool = db.query_row("select exists(select 1 from agent_note_patch_decisions where conversation_id=?1 and proposal_id=?2 and action='undo' and coalesce(json_extract(result_json,'$.generation'),0)=?3)", params![conversation.to_string(),proposal,accepted.generation], |row|row.get(0)).map_err(|e|e.to_string())?;
        let matches = accepted.pages.iter().all(|page| {
            note::load(db, &page.page_id).ok().is_some_and(|current| {
                serde_json::to_value(current).ok() == serde_json::to_value(page).ok()
            })
        });
        let pending_manual = super::manual_pending(db, &accepted.pages)?;
        (
            if undone { "undo" } else { "accept" }.into(),
            matches,
            accepted.generation,
            !undone
                && matches
                && !pending_manual
                && !accepted.originals.iter().any(|note| note.manual_edited),
        )
    } else {
        (
            current
                .as_ref()
                .map_or("pending".into(), |r| r.state.clone()),
            current.as_ref().is_some_and(|r| r.baseline_matches),
            current.as_ref().map_or(generation, |r| r.generation),
            current.as_ref().is_some_and(|r| r.whole_undo_available),
        )
    };
    Ok(Some(decisions::Review {
        persistence_version: current
            .as_ref()
            .map_or(1, |review| review.persistence_version),
        proposal_id: proposal.into(),
        generation,
        pages: notes
            .iter()
            .map(super::views::page_review)
            .collect::<Result<_, _>>()?,
        state,
        baseline_matches,
        operation_generation,
        whole_undo_available,
    }))
}
