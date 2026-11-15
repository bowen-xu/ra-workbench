//! Pure reading, snapshot and preview values; no repository or runtime imports.
use crate::ai::reading_contracts::{LlmModelSelection, LlmModelSelectionDraft};
#[cfg(test)]
pub use research_workbench_native_sdk::paper_context::PaperAiReadingRect;
pub use research_workbench_native_sdk::paper_context::{
    PaperAiReadingAttachmentStatus, PaperAiReadingContextItem, PaperAiReadingImageRequest,
    PaperAiReadingRegion,
};
pub(crate) use research_workbench_native_sdk::paper_context::{
    PaperContextAttachment, PaperContextItem,
};
pub(crate) use research_workbench_native_sdk::paper_context_preview::{
    PaperContextError, PaperContextPreview, PaperContextRequest, identity,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub use research_workbench_native_sdk::paper_context_prepare::PaperAiReadingSessionSummary;

pub use research_workbench_native_sdk::reading_history::contracts::PaperAiReadingMessage;

pub use research_workbench_native_sdk::reading_history::contracts::PaperAiReadingTurn;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperAiReadingPrepareResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<PaperAiReadingSessionSummary>,
    pub attachment_statuses: Vec<PaperAiReadingAttachmentStatus>,
    pub context_items: Vec<PaperAiReadingContextItem>,
    pub image_requests: Vec<PaperAiReadingImageRequest>,
    pub text_item_count: usize,
    pub table_item_count: usize,
    pub picture_item_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<LlmModelSelection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaperAiReadingImageAssetDraft {
    pub context_item_id: String,
    pub data_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(
    dead_code,
    reason = "legacy renderer payload retained while paper reading uses shared Agent submissions"
)]
pub struct PaperAiReadingTurnDraft {
    pub client_request_id: Option<String>,
    pub session_id: Option<String>,
    pub bootstrap_kind: Option<String>,
    pub source_workflow_id: Option<String>,
    pub concept_retrieval_trace_id: Option<String>,
    pub paper_id: String,
    pub attachment_ids: Vec<String>,
    pub user_message: String,
    pub model_selection: LlmModelSelectionDraft,
    pub image_assets: Vec<PaperAiReadingImageAssetDraft>,
}

#[allow(
    unused_imports,
    reason = "retain the original pure reading contract namespace"
)]
pub use research_workbench_native_sdk::reading_history::contracts::PaperReadingStructuredHistoryOption;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "legacy stream wire shape retained for renderer migration"
)]
pub struct PaperAiReadingStreamEvent {
    pub client_request_id: String,
    pub paper_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub ordinal: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    pub accumulated: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "legacy steer wire shape retained for renderer migration"
)]
pub struct PaperAiReadingSteerResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<PaperAiReadingMessage>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "legacy cancel wire shape retained for renderer migration"
)]
pub struct PaperAiReadingCancelResult {
    pub accepted: bool,
    pub resubmit_messages: Vec<PaperAiReadingMessage>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPaperContext {
    pub(crate) attachment_statuses: Vec<PaperAiReadingAttachmentStatus>,
    pub(crate) context_items: Vec<PaperAiReadingContextItem>,
    pub(crate) image_requests: Vec<PaperAiReadingImageRequest>,
    pub(crate) text_item_count: usize,
    pub(crate) table_item_count: usize,
    pub(crate) picture_item_count: usize,
}

impl From<research_workbench_native_sdk::paper_context::PreparedPaperContextV1>
    for PreparedPaperContext
{
    fn from(value: research_workbench_native_sdk::paper_context::PreparedPaperContextV1) -> Self {
        Self {
            attachment_statuses: value.attachment_statuses,
            context_items: value.context_items,
            image_requests: value.image_requests,
            text_item_count: value.text_item_count,
            table_item_count: value.table_item_count,
            picture_item_count: value.picture_item_count,
        }
    }
}
impl PreparedPaperContext {
    pub(crate) fn into_reading_result(
        self,
        session: Option<PaperAiReadingSessionSummary>,
        model: Option<LlmModelSelection>,
    ) -> PaperAiReadingPrepareResult {
        PaperAiReadingPrepareResult {
            session,
            attachment_statuses: self.attachment_statuses,
            context_items: self.context_items,
            image_requests: self.image_requests,
            text_item_count: self.text_item_count,
            table_item_count: self.table_item_count,
            picture_item_count: self.picture_item_count,
            model,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperContextSnapshot {
    pub(crate) id: String,
    pub(crate) paper_id: String,
    pub(crate) context_hash: String,
    pub(crate) context_schema_version: i64,
    pub(crate) attachments: Vec<PaperContextAttachment>,
    pub(crate) items: Vec<PaperContextItem>,
    pub(crate) created_at: String,
}

impl PaperContextSnapshot {
    pub(crate) fn into_reading_result(mut self) -> PaperAiReadingPrepareResult {
        let attachment_order = self
            .attachments
            .iter()
            .map(|attachment| (attachment.attachment_id.as_str(), attachment.ordinal))
            .collect::<std::collections::HashMap<_, _>>();
        self.items.sort_by_key(|item| {
            (
                attachment_order
                    .get(item.attachment_id.as_str())
                    .copied()
                    .unwrap_or(i64::MAX),
                item_kind_order(&item.kind),
                item.id.clone(),
            )
        });
        let context_items = self
            .items
            .into_iter()
            .map(|item| PaperAiReadingContextItem {
                id: item.id,
                paper_id: self.paper_id.clone(),
                attachment_id: item.attachment_id,
                content_sha256: item.content_sha256,
                parse_run_id: item.parse_run_id,
                raw_ref: item.raw_ref,
                kind: item.kind,
                label: item.label,
                page_index: item.page_index,
                regions: item.regions,
                charspan: item.charspan,
                source_text_hash: item.source_text_hash,
                text: item.text,
                table_json: item.table_json,
            })
            .collect::<Vec<_>>();
        let image_requests = context_items
            .iter()
            .filter(|item| item.kind == "picture")
            .filter_map(|item| {
                Some(PaperAiReadingImageRequest {
                    context_item_id: item.id.clone(),
                    attachment_id: item.attachment_id.clone(),
                    page_index: item.page_index?,
                    regions: item.regions.clone(),
                    label: item.label.clone(),
                })
            })
            .collect::<Vec<_>>();
        PaperAiReadingPrepareResult {
            session: None,
            attachment_statuses: self
                .attachments
                .into_iter()
                .map(|attachment| PaperAiReadingAttachmentStatus {
                    display_name: attachment.attachment_id.clone(),
                    attachment_id: attachment.attachment_id,
                    content_sha256: attachment.content_sha256,
                    parse_run_id: Some(attachment.parse_run_id),
                    parse_status: "available".to_owned(),
                })
                .collect(),
            text_item_count: context_items
                .iter()
                .filter(|item| item.kind == "text")
                .count(),
            table_item_count: context_items
                .iter()
                .filter(|item| item.kind == "table")
                .count(),
            picture_item_count: context_items
                .iter()
                .filter(|item| item.kind == "picture")
                .count(),
            context_items,
            image_requests,
            model: None,
        }
    }
}

fn item_kind_order(kind: &str) -> u8 {
    match kind {
        "text" => 0,
        "table" => 1,
        "picture" => 2,
        "group" => 3,
        "key_value" => 4,
        "form" => 5,
        "field" => 6,
        "field_region" => 7,
        _ => u8::MAX,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaperReadingBootstrapMessage {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PaperReadingSessionBootstrap {
    #[expect(
        dead_code,
        reason = "Bootstrap identity and provenance fields remain part of the existing Core input contract."
    )]
    pub(crate) kind: String,
    #[expect(
        dead_code,
        reason = "Bootstrap identity and provenance fields remain part of the existing Core input contract."
    )]
    pub(crate) source_workflow_id: Option<String>,
    #[expect(
        dead_code,
        reason = "Bootstrap identity and provenance fields remain part of the existing Core input contract."
    )]
    pub(crate) source_workflow_revision: Option<i64>,
    pub(crate) messages: Vec<PaperReadingBootstrapMessage>,
    #[expect(
        dead_code,
        reason = "Bootstrap identity and provenance fields remain part of the existing Core input contract."
    )]
    pub(crate) hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceLocation {
    pub(crate) source_id: String,
    pub(crate) id: String,
    pub(crate) attachment_id: String,
    pub(crate) regions: Vec<PaperAiReadingRegion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareRequest {
    pub(crate) paper_id: String,
    pub(crate) attachment_ids: Vec<String>,
    #[serde(default)]
    pub(crate) image_assets: Vec<PaperAiReadingImageAssetDraft>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preparation {
    pub(crate) schema_version: u8,
    pub(crate) paper_id: String,
    pub(crate) snapshot_id: String,
    pub(crate) context_hash: String,
}

impl PrepareRequest {
    pub(crate) fn validate(&self) -> Result<(), PaperContextError> {
        if !identity(&self.paper_id)
            || self.attachment_ids.len() > 64
            || self.attachment_ids.iter().any(|id| !identity(id))
            || self.attachment_ids.iter().collect::<HashSet<_>>().len() != self.attachment_ids.len()
        {
            return Err(PaperContextError::InvalidInput);
        }
        Ok(())
    }
}

/// Exact rows used by the existing structured-evidence validator.
pub(crate) type SnapshotEvidenceRow = (String, String, String, i64, String, String);
pub(crate) type SnapshotItemGeometry = (String, String, String, String);
