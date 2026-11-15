//! Adapted metadata policy from OpenAI Codex rust-v0.146.0, Apache-2.0.
//! Source/ref and adapter differences: docs/architecture/codex-model-catalog.md.
use super::super::{error::AiError, model::LlmModelDescriptor};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CatalogModel {
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub visibility: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub default_reasoning_level: Option<String>,
    #[serde(default)]
    pub(super) supported_reasoning_levels: Vec<Effort>,
    #[serde(default)]
    pub(super) service_tiers: Vec<ServiceTier>,
    #[serde(default)]
    pub(super) input_modalities: Vec<String>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_context_window: Option<u64>,
    #[serde(default)]
    pub auto_compact_token_limit: Option<u64>,
    #[serde(
        default,
        deserialize_with = "optional_truncation_policy",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) truncation_policy: Option<TruncationPolicy>,
    #[serde(default = "default_percent")]
    pub effective_context_window_percent: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TruncationPolicy {
    pub mode: TruncationMode,
    pub limit: i64,
}
impl TruncationPolicy {
    pub(crate) fn shared(
        self,
    ) -> Option<research_agent_core::model::tool_output::ToolOutputPolicy> {
        use research_agent_core::model::tool_output::ToolOutputPolicy;
        let limit = usize::try_from(self.limit).ok()?;
        Some(match self.mode {
            TruncationMode::Bytes => ToolOutputPolicy::Bytes(limit),
            TruncationMode::Tokens => ToolOutputPolicy::Tokens(limit),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TruncationMode {
    Bytes,
    Tokens,
}

fn optional_truncation_policy<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TruncationPolicy>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Effort {
    pub effort: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ServiceTier {
    pub id: String,
}
fn default_percent() -> u8 {
    95
}
const MAX_TOKENS: u64 = 9_007_199_254_740_991;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ContextOverride {
    pub context_window_tokens: Option<u64>,
    pub auto_compact_token_limit: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Overrides {
    schema_version: u8,
    models: BTreeMap<String, ContextOverride>,
}
pub(super) fn read_overrides(path: &Path) -> Result<BTreeMap<String, ContextOverride>, AiError> {
    let bytes = match super::cache::read_bounded(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => return Err(AiError::InvalidInput("模型窗口覆盖配置无法读取".into())),
    };
    let value: Overrides = serde_json::from_slice(&bytes)
        .map_err(|_| AiError::InvalidInput("模型窗口覆盖配置无效".into()))?;
    if value.schema_version != 1
        || value.models.len() > 512
        || value.models.iter().any(|(id, value)| {
            !safe_text(id, 240)
                || [value.context_window_tokens, value.auto_compact_token_limit]
                    .iter()
                    .flatten()
                    .any(|tokens| *tokens == 0 || *tokens > MAX_TOKENS)
        })
    {
        return Err(AiError::InvalidInput("模型窗口覆盖配置无效".into()));
    }
    Ok(value.models)
}
fn safe_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
pub(super) fn validate(models: &[CatalogModel]) -> Result<(), AiError> {
    let unique: BTreeSet<_> = models.iter().map(|model| &model.slug).collect();
    if models.len() > 512
        || unique.len() != models.len()
        || models.iter().any(|model| {
            !safe_text(&model.slug, 240)
                || !safe_text(&model.display_name, 512)
                || model.description.len() > 4096
                || !(1..=100).contains(&model.effective_context_window_percent)
                || [
                    model.context_window,
                    model.max_context_window,
                    model.auto_compact_token_limit,
                ]
                .iter()
                .flatten()
                .any(|tokens| *tokens == 0 || *tokens > MAX_TOKENS)
                || model
                    .context_window
                    .zip(model.max_context_window)
                    .is_some_and(|(value, max)| value > max)
                || model.supported_reasoning_levels.len() > 32
                || model.service_tiers.len() > 32
                || model.input_modalities.len() > 16
                || model
                    .input_modalities
                    .iter()
                    .any(|value| !safe_text(value, 64))
                || model
                    .supported_reasoning_levels
                    .iter()
                    .any(|value| !safe_text(&value.effort, 64))
                || model
                    .service_tiers
                    .iter()
                    .any(|value| !safe_text(&value.id, 64))
        })
    {
        return Err(AiError::Provider("invalid_response"));
    }
    Ok(())
}
impl CatalogModel {
    pub(crate) fn capabilities(&self) -> research_agent_core::model::ModelCapabilities {
        research_agent_core::model::ModelCapabilities {
            vision: self.input_modalities.iter().any(|value| value == "image"),
            tools: true,
            parallel_tools: false,
            reasoning_efforts: self.descriptor(false).supported_reasoning_efforts,
        }
    }
    pub(super) fn with_override(mut self, value: Option<&ContextOverride>) -> Self {
        if let Some(value) = value {
            if let Some(window) = value.context_window_tokens {
                // An override cannot invent a larger maximum when metadata is missing.
                self.context_window = self
                    .max_context_window
                    .or(self.context_window)
                    .map(|max| window.min(max));
            }
            if let Some(trigger) = value.auto_compact_token_limit {
                self.auto_compact_token_limit = Some(trigger);
            }
        }
        self
    }
    pub(crate) fn window(&self) -> Option<u64> {
        self.context_window
    }
    pub(crate) fn descriptor(&self, is_default: bool) -> LlmModelDescriptor {
        let supported = self
            .supported_reasoning_levels
            .iter()
            .filter(|level| {
                matches!(
                    level.effort.as_str(),
                    "none" | "low" | "medium" | "high" | "xhigh" | "max"
                )
            })
            .map(|level| level.effort.clone())
            .collect::<Vec<_>>();
        let default = self
            .default_reasoning_level
            .as_ref()
            .filter(|effort| supported.contains(effort))
            .cloned()
            .or_else(|| supported.first().cloned());
        LlmModelDescriptor {
            id: self.slug.clone(),
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            is_default,
            default_reasoning_effort: default,
            supported_reasoning_efforts: supported,
            supports_fast_mode: self.service_tiers.iter().any(|tier| tier.id == "priority"),
            context_window_tokens: self.window().map(|tokens| {
                (u128::from(tokens) * u128::from(self.effective_context_window_percent) / 100)
                    as i64
            }),
            context_window_source: self.window().map(|_| "provider_catalog".into()),
        }
    }
}
