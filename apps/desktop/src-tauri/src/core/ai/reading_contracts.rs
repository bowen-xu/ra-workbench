//! Pure values shared by existing model selection and paper-image adapters.
use serde::Serialize;

pub use research_workbench_native_sdk::generation::LlmModelSelectionDraft;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelSelection {
    pub provider_id: String,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

pub(crate) use research_workbench_native_sdk::generation::provider::StoredImageAsset;
