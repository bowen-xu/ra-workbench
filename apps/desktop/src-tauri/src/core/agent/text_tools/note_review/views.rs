//! UI-only rich document projection; model tools continue to expose authoring text.
use super::super::{decisions::PageReview, note::PendingNote};
use super::{baseline_document, chunks, metadata_pending};
pub(in crate::agent::text_tools) fn page_review(note: &PendingNote) -> Result<PageReview, String> {
    let page = note
        .candidate
        .as_ref()
        .or(note.baseline.as_ref())
        .ok_or("missing page")?;
    let kind = match (&note.baseline, &note.candidate) {
        (None, _) => "create",
        (_, None) => "trash",
        (Some(before), Some(after)) if before.parent_id != after.parent_id => "move",
        _ => "update",
    };
    let review_document = baseline_document(note);
    let after_document = note.candidate.as_ref().map(|note| note.document.clone());
    let chunks = chunks::chunks(
        &review_document,
        &after_document
            .clone()
            .unwrap_or_else(|| chunks::empty(&page.page_id)),
    )?;
    let mut accepted = super::baseline_metadata(note);
    let candidate_metadata = super::metadata(note.candidate.as_ref());
    accepted.title = candidate_metadata.title.clone();
    Ok(PageReview {
        page_id: page.page_id.clone(),
        path: note.path.clone(),
        title: page.title.clone(),
        kind: kind.into(),
        expected_revision: note
            .saved
            .as_ref()
            .or(note.baseline.as_ref())
            .map(|note| note.revision),
        recovery: None,
        before: note
            .baseline
            .as_ref()
            .map(|note| crate::notes::text_view::render(&note.document, &note.title))
            .transpose()?,
        after: note
            .candidate
            .as_ref()
            .map(|note| crate::notes::text_view::render(&note.document, &note.title))
            .transpose()?,
        before_document: note.baseline.as_ref().map(|note| note.document.clone()),
        after_document,
        review_document: Some(review_document),
        chunks,
        metadata_pending: metadata_pending(note),
        structural_metadata_pending: metadata_pending(note) && accepted != candidate_metadata,
    })
}

pub(in crate::agent::text_tools) fn with_authority(
    db: &rusqlite::Connection,
    note: &PendingNote,
) -> Result<PageReview, String> {
    let mut page = page_review(note)?;
    if !super::persistence::matches(db, note) {
        let current = super::super::note::load(db, &page.page_id)?;
        page.recovery = Some(super::super::decisions::PageRecovery {
            current_revision: current.revision,
            status: current.status,
        });
    }
    Ok(page)
}
