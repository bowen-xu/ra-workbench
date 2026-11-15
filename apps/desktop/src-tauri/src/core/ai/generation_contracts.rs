//! Shared provider input/output data used by generation adapters.
use serde_json::Value;

/// Only the values the paper workflow persists after shared execution.
pub(crate) struct PaperGenerationResponse {
    pub content: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

pub(crate) use research_workbench_native_sdk::generation::provider::{
    ProviderImage, ProviderInput, ProviderMessage, ProviderMessageRole,
};

#[derive(Clone)]
pub(crate) struct VisionGenerationResponse {
    pub(crate) content: String,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    // Legacy transport metadata retains its original dead-code compatibility.
    #[allow(dead_code)]
    pub(crate) context_window_tokens: Option<i64>,
    #[allow(dead_code)]
    pub(crate) context_window_source: Option<String>,
    #[allow(dead_code)]
    pub(in crate::ai) raw_response: Value,
}
