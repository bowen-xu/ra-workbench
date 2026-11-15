//! Generic Agent evidence adapters over the selected immutable reading manifest.

pub(crate) use crate::ai::paper_reading_contracts::{
    PaperContextError as Error, PaperContextPreview as Preview, PaperContextRequest as Request,
    SourceLocation,
};
use crate::{
    ai::{paper_reading_source::PaperReadingSource, reading_support::PaperReadingImages},
    storage::AppStore,
};
use research_agent_core::submission::{ContextSourceKind, ContextText, SubmissionImage};
use rusqlite::Connection;
use std::collections::HashSet;
const MAX_ITEMS: usize = 20_000;
pub(crate) fn recanonicalize_snapshot(
    source: PaperReadingSource,
    db: &mut Connection,
    id: &str,
) -> Result<(String, String), Error> {
    (source.recanonicalize)(db, id).map_err(|_| Error::Corrupt)
}
fn materialize_in(
    source: PaperReadingSource,
    db: &Connection,
    request: &Request,
) -> Result<Preview, Error> {
    (source.preview)(db, request)
}
/// Convert one explicitly selected immutable snapshot into provider-neutral
/// Agent evidence. Picture bytes remain in the separate image manifest; this
/// function neither reads the content store nor silently converts images.
pub(crate) fn materialize_submission_context(
    source: PaperReadingSource,
    db: &Connection,
    paper_id: &str,
    snapshot_id: &str,
    expected_context_hash: &str,
) -> Result<Vec<ContextText>, Error> {
    let preview = materialize_in(
        source,
        db,
        &Request {
            paper_id: paper_id.to_owned(),
            snapshot_id: snapshot_id.to_owned(),
            expected_context_hash: expected_context_hash.to_owned(),
        },
    )?;
    submission_context_from_preview(preview)
}

/// Assemble the original provider-neutral evidence after the selected reader
/// validates the immutable snapshot in the caller's transaction.
pub(crate) fn submission_context_from_preview(preview: Preview) -> Result<Vec<ContextText>, Error> {
    preview
        .items
        .into_iter()
        .filter_map(|entry| {
            let text = match entry.item.kind.as_str() {
                "text" => entry.item.text.clone(),
                "table" => entry.item.table_json.as_ref().map(ToString::to_string),
                _ => None,
            }?;
            Some((entry, text))
        })
        .map(|(entry, text)| {
            let anchor = serde_json::to_string(&serde_json::json!({
                "paperId": preview.paper_id.clone(),
                "snapshotId": preview.snapshot_id.clone(),
                "attachmentId": entry.item.attachment_id,
                "parseRunId": entry.item.parse_run_id,
                "kind": entry.item.kind,
                "rawRef": entry.item.raw_ref,
                "pageIndex": entry.item.page_index,
            }))
            .map_err(|_| Error::Corrupt)?;
            Ok(ContextText {
                source_id: entry.source_id,
                kind: ContextSourceKind::Paper,
                resource_id: preview.paper_id.clone(),
                revision: preview.context_hash.clone(),
                anchor: Some(anchor),
                text,
            })
        })
        .collect()
}

/// Variant used by migration adapters: retain only the snapshot items that a
/// legacy turn explicitly recorded as evidence.
pub(crate) fn materialize_submission_context_for_items(
    source: PaperReadingSource,
    db: &Connection,
    paper_id: &str,
    snapshot_id: &str,
    expected_context_hash: &str,
    item_ids: &HashSet<String>,
) -> Result<Vec<ContextText>, Error> {
    if item_ids.is_empty() || item_ids.len() > MAX_ITEMS {
        return Err(Error::InvalidInput);
    }
    let preview = materialize_in(
        source,
        db,
        &Request {
            paper_id: paper_id.to_owned(),
            snapshot_id: snapshot_id.to_owned(),
            expected_context_hash: expected_context_hash.to_owned(),
        },
    )?;
    let mut contexts = Vec::new();
    let mut found = HashSet::new();
    for entry in preview.items {
        if !item_ids.contains(&entry.item.id) {
            continue;
        }
        found.insert(entry.item.id.clone());
        let text = match entry.item.kind.as_str() {
            "text" => entry.item.text.clone(),
            "table" => entry.item.table_json.as_ref().map(ToString::to_string),
            _ => None,
        };
        let Some(text) = text else { continue };
        let anchor = serde_json::to_string(&serde_json::json!({
            "paperId": preview.paper_id.clone(),
            "snapshotId": preview.snapshot_id.clone(),
            "attachmentId": entry.item.attachment_id,
            "parseRunId": entry.item.parse_run_id,
            "kind": entry.item.kind,
            "rawRef": entry.item.raw_ref,
            "pageIndex": entry.item.page_index,
        }))
        .map_err(|_| Error::Corrupt)?;
        contexts.push(ContextText {
            source_id: entry.source_id,
            kind: ContextSourceKind::Paper,
            resource_id: preview.paper_id.clone(),
            revision: preview.context_hash.clone(),
            anchor: Some(anchor),
            text,
        });
    }
    if found.len() != item_ids.len() {
        return Err(Error::Conflict);
    }
    Ok(contexts)
}

/// Materialize only the picture bytes already bound to this immutable
/// snapshot. The content store is never resolved from a renderer-provided
/// path, and missing/corrupt bytes fail closed.
#[path = "agent_preview/submission_images.rs"]
mod submission_images;
pub(crate) use submission_images::submission_image_references_from_preview;

pub(crate) fn materialize_submission_images_for_items(
    source: PaperReadingSource,
    db: &Connection,
    store: &AppStore,
    paper_id: &str,
    snapshot_id: &str,
    expected_context_hash: &str,
    item_ids: Option<&HashSet<String>>,
) -> Result<Vec<SubmissionImage>, Error> {
    let preview = materialize_in(
        source,
        db,
        &Request {
            paper_id: paper_id.to_owned(),
            snapshot_id: snapshot_id.to_owned(),
            expected_context_hash: expected_context_hash.to_owned(),
        },
    )?;
    submission_images_from_preview(preview, db, store, item_ids)
}

/// Image byte access remains Core-owned and follows the original preview read.
pub(crate) fn submission_images_from_preview(
    preview: Preview,
    db: &Connection,
    store: &AppStore,
    item_ids: Option<&HashSet<String>>,
) -> Result<Vec<SubmissionImage>, Error> {
    let mut images = Vec::new();
    for item in preview.items.into_iter().filter(|item| {
        item.item.kind == "picture" && item_ids.is_none_or(|ids| ids.contains(&item.item.id))
    }) {
        let hash = item.item.image_content_sha256.ok_or(Error::Unavailable)?;
        let image = PaperReadingImages::new(db, store)
            .read(&hash)
            .map_err(|_| Error::Unavailable)?;
        images.push(SubmissionImage {
            source_id: item.source_id,
            media_type: image.mime_type,
            sha256: hash,
            bytes: image.bytes,
        });
    }
    Ok(images)
}

/// Rebuild the navigable geometry for sources admitted to one historical
/// submission. Callers supply the persisted submission source ids; aliases or
/// renderer state never grant access to additional snapshot items.
pub(crate) fn materialize_source_locations(
    source: PaperReadingSource,
    db: &Connection,
    paper_id: &str,
    snapshot_id: &str,
    expected_context_hash: &str,
    allowed_source_ids: &HashSet<String>,
) -> Result<Vec<SourceLocation>, Error> {
    validate_source_location_ids(allowed_source_ids)?;
    let preview = materialize_in(
        source,
        db,
        &Request {
            paper_id: paper_id.to_owned(),
            snapshot_id: snapshot_id.to_owned(),
            expected_context_hash: expected_context_hash.to_owned(),
        },
    )?;
    source_locations_from_preview(preview, allowed_source_ids)
}

pub(crate) fn validate_source_location_ids(allowed: &HashSet<String>) -> Result<(), Error> {
    if allowed.is_empty() || allowed.len() > MAX_ITEMS {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

/// Call after the original allowed-ID validation and snapshot read, in that order.
pub(crate) fn source_locations_from_preview(
    preview: Preview,
    allowed_source_ids: &HashSet<String>,
) -> Result<Vec<SourceLocation>, Error> {
    let locations: Vec<_> = preview
        .items
        .into_iter()
        .filter(|entry| allowed_source_ids.contains(&entry.source_id))
        .map(|entry| SourceLocation {
            source_id: entry.source_id,
            id: entry.item.id,
            attachment_id: entry.item.attachment_id,
            regions: entry.item.regions,
        })
        .collect();
    if locations.len() != allowed_source_ids.len() {
        return Err(Error::Conflict);
    }
    Ok(locations)
}
