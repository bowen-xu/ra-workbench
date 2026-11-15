//! One statically selected reading capability; function values are not persisted.
//! Raw SQL borrows are only for App's internal repository composition. Domain
//! policy receives closed repositories and Core facts/images instead.
use super::{
    error::AiError, reading_contracts::StoredImageAsset, reading_support::PaperReadingImages,
};
use crate::ai::paper_reading_contracts::*;
use rusqlite::Connection;
use std::collections::BTreeMap;
type PersistSnapshot = fn(
    &mut Connection,
    &str,
    &PaperAiReadingPrepareResult,
    &BTreeMap<String, StoredImageAsset>,
) -> Result<PaperContextSnapshot, AiError>;
type RecanonicalizeSnapshot = fn(&mut Connection, &str) -> Result<(String, String), AiError>;
type LatestSnapshot = fn(&Connection, &str) -> Result<Option<(String, String)>, AiError>;
type SaveReadingImages = fn(
    &PaperReadingImages<'_>,
    &[PaperAiReadingImageAssetDraft],
) -> Result<BTreeMap<String, StoredImageAsset>, AiError>;

#[derive(Clone, Copy)]
pub(crate) struct PaperReadingSource {
    pub(crate) attachment_path:
        fn(&Connection, &std::path::Path, &str) -> Result<std::path::PathBuf, String>,
    pub(crate) prepare:
        fn(&Connection, &str, &[String]) -> Result<PaperAiReadingPrepareResult, AiError>,
    pub(crate) snapshot_ready: fn(&PreparedPaperContext) -> bool,
    pub(crate) build: fn(&Connection, &str, &[String]) -> Result<PreparedPaperContext, AiError>,
    pub(crate) persist: PersistSnapshot,
    pub(crate) load: fn(&Connection, &str) -> Result<PaperContextSnapshot, AiError>,
    pub(crate) recanonicalize: RecanonicalizeSnapshot,
    pub(crate) preview:
        fn(&Connection, &PaperContextRequest) -> Result<PaperContextPreview, PaperContextError>,
    pub(crate) latest: LatestSnapshot,
    pub(crate) snapshot_hash: fn(&Connection, &str, &str) -> Result<Option<String>, AiError>,
    pub(crate) validation_latest_snapshot: fn(&Connection, &str) -> Result<Option<String>, AiError>,
    pub(crate) snapshot_evidence_rows:
        fn(&Connection, &str) -> Result<Vec<SnapshotEvidenceRow>, AiError>,
    pub(crate) item_geometry:
        fn(&Connection, &str, &str) -> Result<Option<SnapshotItemGeometry>, AiError>,
    pub(crate) frozen_evidence_belongs: fn(&Connection, &str, &str) -> Result<bool, AiError>,
    pub(crate) save_images: SaveReadingImages,
    pub(crate) validate_images: fn(
        &PaperAiReadingPrepareResult,
        &BTreeMap<String, StoredImageAsset>,
    ) -> Result<(), AiError>,
}
