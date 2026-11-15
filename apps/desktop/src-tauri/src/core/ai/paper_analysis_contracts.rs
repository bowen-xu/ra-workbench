//! Existing analysis and template-reference wire values; no runtime imports.
use crate::ai::paper_reading_contracts::PaperAiReadingImageAssetDraft;
use crate::ai::reading_contracts::LlmModelSelectionDraft;
use serde::Deserialize;

pub use research_workbench_native_sdk::candidate_contracts::PaperTemplateRef;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaperAnalysisStartDraft {
    #[serde(default)]
    pub regenerate: bool,
    pub client_request_id: String,
    pub paper_id: String,
    pub attachment_ids: Vec<String>,
    pub overview_profile_selection_revision: i64,
    pub paper_template_ref: Option<PaperTemplateRef>,
    pub model_selection: Option<LlmModelSelectionDraft>,
    pub image_assets: Vec<PaperAiReadingImageAssetDraft>,
    #[serde(default)]
    pub skill_source_ids: Vec<String>,
    #[serde(default)]
    pub resource_inputs: Vec<research_agent_core::user_input::UserInput>,
}

#[allow(unused_imports)] // Preserve the original named wire-contract entry.
pub use research_workbench_native_sdk::analysis_history::contracts::{
    PaperAnalysisAttempt, PaperAnalysisMessage, PaperAnalysisMessageKind, PaperAnalysisTurn,
    PaperAnalysisWorkflow,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DoclingPaperOverviewDraft {
    pub paper_id: String,
    pub attachment_ids: Vec<String>,
    pub context_snapshot_id: Option<String>,
    pub overview_profile_selection_revision: i64,
    pub paper_template_ref: Option<PaperTemplateRef>,
    pub model_selection: Option<LlmModelSelectionDraft>,
    pub image_assets: Vec<PaperAiReadingImageAssetDraft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_conversation_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_request_id: Option<uuid::Uuid>,
}

pub(crate) struct AttemptSuccess<'a> {
    pub(crate) ai_run_id: &'a str,
    pub(crate) candidate_id: &'a str,
    pub(crate) input_hash: Option<&'a str>,
    pub(crate) output_hash: Option<&'a str>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
}
