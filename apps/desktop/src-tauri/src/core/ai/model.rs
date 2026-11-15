pub(crate) use super::candidate_contracts::{
    AiEvidenceBundle, AiEvidenceItem, AiMatrixSynthesisContext, AiOverviewProfileContext,
    AiReadingProfileRef,
};
pub(crate) use super::reading_contracts::{LlmModelSelection, LlmModelSelectionDraft};
use serde::{Deserialize, Serialize};

use super::provider::ProviderCapabilities;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRunProgressEvent {
    pub run_id: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderSettingsDraft {
    pub provider_id: String,
    pub enabled: bool,
    pub base_url: String,
    pub region: String,
    pub model_id: String,
    pub thinking_mode: String,
    pub timeout_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderSettings {
    pub provider_id: String,
    pub provider_kind: String,
    pub enabled: bool,
    pub base_url: String,
    pub region: String,
    pub model_id: String,
    pub thinking_mode: String,
    pub timeout_ms: i64,
    pub is_cloud: bool,
    pub credential_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_mask: Option<String>,
    pub updated_at: String,
}

pub use research_workbench_native_sdk::generation::{
    AiMatrixSynthesisContextDraft, AiScopeItemDraft, AiTaskSpecDraft,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiScopeItem {
    pub id: String,
    pub ordinal: i64,
    pub item_kind: String,
    pub object_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_revision: Option<i64>,
    pub snapshot_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskPreview {
    pub task_kind: String,
    pub provider_id: String,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub is_cloud: bool,
    pub scope_hash: String,
    pub input_item_count: i64,
    pub scope_items: Vec<AiScopeItem>,
    pub requires_cloud_consent: bool,
    pub evidence_preview: Vec<AiEvidencePreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview_profile: Option<AiOverviewProfileContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion_candidate_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix_synthesis_context: Option<AiMatrixSynthesisContext>,
}

pub use research_workbench_native_sdk::generation::evidence::AiEvidencePreview;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOverviewRetryContext {
    pub candidate_id: String,
    pub base_revision: i64,
    pub target_fields: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAttentionFeedbackDraft {
    pub candidate_id: String,
    pub candidate_revision: i64,
    pub suggestion_id: String,
    pub feedback_kind: String,
    pub note: String,
    pub idempotency_key: String,
}

pub use research_workbench_native_sdk::candidate_review::items::{
    AiSuggestionItemDecisionDraft, AiSuggestionResolutionDraft,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCandidateSummaryVersionDraft {
    pub candidate_id: String,
    pub candidate_revision: i64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCandidateSummarySelectionDraft {
    pub candidate_id: String,
    pub candidate_revision: i64,
    pub summary_version_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCandidateSummaryDiscardDraft {
    pub candidate_id: String,
    pub candidate_revision: i64,
    pub keep_summary_version_id: String,
}

pub use research_workbench_native_sdk::paper_confirmation::contracts::{
    AiOverviewConfirmationBatch, AiOverviewConfirmationResult, AiOverviewConfirmationUndoDraft,
    AiOverviewFieldConfirmationDraft, AiPaperReviewConfirmationBatch,
    AiPaperReviewConfirmationDraft, AiPaperReviewConfirmationResult,
    AiPaperReviewConfirmationUndoDraft, AiPaperReviewMetadataConfirmation,
    AiPaperReviewOverviewTargetDraft,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRunStage {
    pub id: String,
    pub run_id: String,
    pub stage_kind: String,
    pub attempt: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRun {
    pub id: String,
    pub task_kind: String,
    pub status: String,
    pub provider_id: String,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub pipeline_version: String,
    pub prompt_version: String,
    pub output_schema_version: i64,
    pub user_goal: String,
    pub scope_hash: String,
    pub input_item_count: i64,
    pub max_input_tokens: i64,
    pub max_output_tokens: i64,
    pub timeout_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_consent_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_consented_at: Option<String>,
    pub cancel_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_miss_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub updated_at: String,
    pub scope_items: Vec<AiScopeItem>,
    pub stages: Vec<AiRunStage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview_retry: Option<AiOverviewRetryContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview_profile: Option<AiOverviewProfileContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion_candidate_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix_synthesis_context: Option<AiMatrixSynthesisContext>,
    /// Shared Agent identity for legacy overview/suggestion projections.
    /// Omitted for runs that have not entered the migration bridge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_turn_outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_candidate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_terminal_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderOverview {
    pub settings: AiProviderSettings,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderTestResult {
    pub provider_id: String,
    pub model_id: String,
    pub reachable: bool,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmProviderConfigDraft {
    pub provider_id: String,
    pub enabled: bool,
    pub base_url: String,
    pub timeout_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderConfig {
    pub provider_id: String,
    pub display_name: String,
    pub auth_mode: String,
    pub protocol: String,
    pub model_kinds: Vec<String>,
    pub default_base_urls: Vec<String>,
    pub base_url: String,
    pub uses_default_base_url: bool,
    pub enabled: bool,
    pub timeout_ms: i64,
    pub credential_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_mask: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmModelRouteDraft {
    pub route_kind: String,
    pub route_id: String,
    pub selection: Option<LlmModelSelectionDraft>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelRoute {
    pub route_kind: String,
    pub route_id: String,
    pub label: String,
    pub model_kind: String,
    pub selection: Option<LlmModelSelection>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmConfiguration {
    pub providers: Vec<LlmProviderConfig>,
    pub default_models: Vec<LlmModelRoute>,
    pub use_cases: Vec<LlmModelRoute>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderModels {
    pub provider_id: String,
    pub models: Vec<LlmModelDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<LlmCatalogStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmCatalogStatus {
    pub source: String,
    pub stale: bool,
    pub fetched_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub is_default: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
    pub supported_reasoning_efforts: Vec<String>,
    pub supports_fast_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptAccountStatus {
    pub runtime_available: bool,
    pub configured: bool,
    pub auth_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptLoginStart {
    pub login_id: String,
    pub auth_url: String,
}

pub(crate) use crate::ai::paper_reading_contracts::{
    PaperAiReadingContextItem, PaperAiReadingPrepareResult, PaperAiReadingTurn,
};

pub(crate) use crate::ai::paper_analysis_contracts::DoclingPaperOverviewDraft;

#[cfg(test)]
pub(crate) use crate::ai::paper_reading_contracts::{
    PaperAiReadingAttachmentStatus, PaperAiReadingImageRequest, PaperAiReadingMessage,
    PaperAiReadingRect, PaperAiReadingRegion,
};

pub use research_workbench_native_sdk::candidate_review::contracts::{
    AiAttentionFeedback, AiCandidate, AiCandidateRevision, AiCandidateRevisionDraft,
    AiCandidateSummaryVersion, AiCitationAssessment, AiCitationAssessmentDraft,
    AiOverviewFieldConfirmation, AiReviewDecision, AiReviewDecisionDraft, AiSuggestionItemDecision,
};
