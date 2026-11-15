//! Core reading callers consume explicitly selected Native capabilities.
#[path = "agent_preview.rs"]
pub(crate) mod agent_preview;
pub(crate) mod prepare;
pub(crate) mod preview;
pub(crate) use crate::ai::paper_reading_contracts::{PaperContextSnapshot, PreparedPaperContext};
pub(crate) use crate::ai::reading_contracts::StoredImageAsset;
use crate::ai::{error::AiError, paper_reading_source::PaperReadingSource};
use rusqlite::Connection;
use std::collections::BTreeMap;
pub(crate) fn build_paper_context(
    source: PaperReadingSource,
    db: &Connection,
    paper: &str,
    attachments: &[String],
) -> Result<PreparedPaperContext, AiError> {
    (source.build)(db, paper, attachments)
}
pub(crate) fn persist_snapshot(
    source: PaperReadingSource,
    db: &mut Connection,
    paper: &str,
    prepared: &crate::ai::paper_reading_contracts::PaperAiReadingPrepareResult,
    assets: &BTreeMap<String, StoredImageAsset>,
) -> Result<PaperContextSnapshot, AiError> {
    (source.persist)(db, paper, prepared, assets)
}
pub(crate) fn load_snapshot(
    source: PaperReadingSource,
    db: &Connection,
    id: &str,
) -> Result<PaperContextSnapshot, AiError> {
    (source.load)(db, id)
}
