//! Read immutable submission image references without opening the image bytes.
use super::{Error, Preview};
use crate::ai::reading_support::PaperReadingFacts;
use rusqlite::Connection;

/// Reuse the original metadata queries after a static or Native preview.
pub(crate) fn submission_image_references_from_preview(
    preview: Preview,
    db: &Connection,
) -> Result<Vec<research_agent_core::submission::SubmissionImageReference>, Error> {
    let mut references = Vec::new();
    for item in preview
        .items
        .into_iter()
        .filter(|entry| entry.item.kind == "picture")
    {
        let hash = item.item.image_content_sha256.ok_or(Error::Unavailable)?;
        let metadata = PaperReadingFacts::new(db)
            .image_metadata(&hash)
            .map_err(|_| Error::Unavailable)?;
        references.push(research_agent_core::submission::SubmissionImageReference {
            source_id: item.source_id,
            content_sha256: hash,
            media_type: metadata.mime_type,
            byte_size: metadata.byte_size,
            ordinal: references.len() as u32,
        });
    }
    research_agent_core::submission::validate_image_references(&references)
        .map_err(|_| Error::Corrupt)?;
    Ok(references)
}
