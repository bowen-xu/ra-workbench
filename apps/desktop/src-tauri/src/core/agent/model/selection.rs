//! Renderer-safe model choice shared by host submission commands.
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    pub provider_id: String,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
}

impl From<ModelSelection> for super::AgentModelSelection {
    fn from(value: ModelSelection) -> Self {
        Self {
            provider_id: value.provider_id,
            model_id: value.model_id,
            reasoning_effort: value.reasoning_effort,
            service_tier: value.service_tier,
        }
    }
}
