use serde::{Deserialize, Serialize};

const MAX_TITLE_LENGTH: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotePage {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literature_paper_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_page_id: Option<String>,
    pub position: i64,
    pub status: String,
    pub is_favorite: bool,
    pub revision: i64,
    pub schema_version: i64,
    pub project_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotePageDraft {
    pub title: String,
    pub parent_page_id: Option<String>,
    pub parent_block_position: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteListQuery {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub parent_page_id: Option<String>,
    pub project_id: Option<String>,
    pub tag_id: Option<String>,
    pub search: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotePageMetadataUpdate {
    pub page_id: String,
    pub title: Option<String>,
    pub is_favorite: Option<bool>,
    pub project_ids: Option<Vec<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotePageMoveRequest {
    pub page_id: String,
    pub parent_page_id: Option<String>,
    pub position: i64,
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteStatusUpdate {
    pub id: String,
    pub status: String,
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotePermanentDeleteRequest {
    pub id: String,
    pub expected_revision: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotePermanentDeleteReport {
    pub scheduled_asset_count: usize,
    pub estimated_reclaimable_bytes: i64,
    pub cleanup_warnings: Vec<String>,
}

pub use research_workbench_native_sdk::note_document::{NoteDocument, NoteDocumentBlock};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteDocumentSnapshot {
    pub page: NotePage,
    pub document: NoteDocument,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteDocumentSaveRequest {
    pub document: NoteDocument,
    pub expected_revision: i64,
    pub save_reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteRevisionSummary {
    pub id: String,
    pub page_id: String,
    pub page_revision: i64,
    pub reason: String,
    pub created_at: String,
    pub is_pinned: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSelectorSegment {
    pub page_index: i64,
    pub page_label: Option<String>,
    pub rects: Vec<SourceRect>,
    pub exact_text: String,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSelectorV2 {
    pub schema_version: i64,
    pub anchor_id: Option<String>,
    pub attachment_sha256: Option<String>,
    pub segments: Vec<SourceSelectorSegment>,
}

impl SourceSelectorV2 {
    pub fn is_valid_for(
        &self,
        page_index: i64,
        attachment_sha256: &str,
        quote_exact: &str,
    ) -> bool {
        if self.schema_version != 2 || !(1..=2).contains(&self.segments.len()) {
            return false;
        }
        if self.attachment_sha256.as_deref() != Some(attachment_sha256) {
            return false;
        }
        if self.segments.first().map(|segment| segment.page_index) != Some(page_index) {
            return false;
        }
        let valid_segments = self.segments.iter().all(|segment| {
            segment.page_index >= 0
                && segment.rects.iter().all(|rect| {
                    rect.x.is_finite()
                        && rect.y.is_finite()
                        && rect.width.is_finite()
                        && rect.height.is_finite()
                        && rect.width > 0.0
                        && rect.height > 0.0
                })
        });
        valid_segments
            && self
                .segments
                .iter()
                .map(|segment| segment.exact_text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
                == quote_exact
            && (self.segments.len() == 1
                || (self.segments[0].page_index - self.segments[1].page_index).abs() == 1)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingNoteDraft {
    pub id: Option<String>,
    pub source_anchor_id: Option<String>,
    pub note_type: String,
    pub body: String,
    pub color: Option<String>,
    pub item_id: String,
    pub attachment_id: String,
    pub page_index: i64,
    pub page_label: Option<String>,
    pub rects: Vec<SourceRect>,
    pub quote_exact: String,
    pub quote_prefix: Option<String>,
    pub quote_suffix: Option<String>,
    pub selector: Option<SourceSelectorV2>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingNoteSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_id: Option<String>,
    pub paper_id: String,
    pub paper_title: String,
    pub attachment_id: String,
    pub content_sha256: String,
    pub page_index: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_label: Option<String>,
    pub rects: serde_json::Value,
    pub quote_exact: String,
    pub quote_prefix: String,
    pub quote_suffix: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<SourceSelectorV2>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingNoteBlock {
    pub id: String,
    pub page_id: String,
    pub note_type: String,
    pub body: String,
    pub color: String,
    pub status: String,
    pub is_favorite: bool,
    pub revision: i64,
    pub source: ReadingNoteSource,
    pub project_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingNoteUpdate {
    pub id: String,
    pub color: Option<String>,
    pub position: Option<SourceRect>,
    pub is_favorite: Option<bool>,
    pub project_ids: Option<Vec<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingNoteListQuery {
    pub status: Option<String>,
    pub project_id: Option<String>,
    pub tag_id: Option<String>,
    pub search: Option<String>,
    pub paper_id: Option<String>,
    pub note_type: Option<String>,
    pub source_state: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSearchQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub object_kinds: Option<Vec<String>>,
    pub project_id: Option<String>,
    pub tag_id: Option<String>,
    pub paper_id: Option<String>,
    pub note_type: Option<String>,
    pub source_state: Option<String>,
    pub is_favorite: Option<bool>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSearchResult {
    pub object_kind: String,
    pub object_id: String,
    pub title: String,
    pub snippet: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ReadingNoteSource>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSearchPage {
    pub items: Vec<NoteSearchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub index_state: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteLinkTarget {
    pub kind: String,
    pub id: String,
    pub label_snapshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_snapshot: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteLinkDraft {
    pub source_page_id: String,
    pub source_block_id: Option<String>,
    pub target: NoteLinkTarget,
    pub relation: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteLink {
    pub id: String,
    pub source_page_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_block_id: Option<String>,
    pub target: NoteLinkTargetOutput,
    pub relation: String,
    pub is_broken: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteLinkTargetOutput {
    pub kind: String,
    pub id: String,
    pub label_snapshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_snapshot: Option<String>,
}

pub(crate) fn validate_status(status: &str) -> bool {
    matches!(status, "active" | "archived" | "trashed")
}

pub(crate) fn validate_title(title: &str) -> Option<String> {
    let normalized = title.trim();
    if normalized.chars().count() > MAX_TITLE_LENGTH {
        return None;
    }
    Some(
        if normalized.is_empty() {
            "无标题"
        } else {
            normalized
        }
        .to_owned(),
    )
}

pub(crate) fn reading_note_type_to_block_type(note_type: &str) -> Option<&'static str> {
    match note_type {
        "highlight" => Some("reading_highlight"),
        "annotation" => Some("reading_annotation"),
        "page_note" => Some("reading_page_note"),
        _ => None,
    }
}
