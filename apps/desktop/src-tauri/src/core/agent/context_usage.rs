//! Sampling-window measurements, distinct from cumulative billing and compact I/O.
use research_agent_core::{
    context::{accounting::AutoCompactionScope, compaction_strategy::CompactionBudget},
    protocol::ModelIdentity,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_SAFE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowUsageSample {
    pub turn_id: Uuid,
    pub request_id: Uuid,
    pub window_id: Option<Uuid>,
    pub model: ModelIdentity,
    pub actual_model: ModelIdentity,
    #[serde(default = "model_observed_by_legacy_sample")]
    pub actual_model_observed: bool,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub estimated_tokens: Option<u64>,
    pub budget: Option<CompactionBudget>,
    pub scope: AutoCompactionScope,
}

fn model_observed_by_legacy_sample() -> bool {
    true
}

impl WindowUsageSample {
    pub fn validate(&self) -> bool {
        !self.turn_id.is_nil()
            && !self.request_id.is_nil()
            && self.window_id.is_none_or(|id| !id.is_nil())
            && [&self.model, &self.actual_model].iter().all(|model| {
                !model.provider_id.trim().is_empty() && !model.model_id.trim().is_empty()
            })
            && self.actual_model.provider_id == self.model.provider_id
            && [self.input_tokens, self.output_tokens, self.estimated_tokens]
                .iter()
                .all(|value| value.is_none_or(|value| value <= MAX_SAFE))
            && self
                .input_tokens
                .zip(self.output_tokens)
                .is_none_or(|(input, output)| {
                    input
                        .checked_add(output)
                        .is_some_and(|total| total <= MAX_SAFE)
                })
            && self.budget.is_none_or(|budget| {
                budget.context_window_tokens <= MAX_SAFE
                    && budget.output_reserve_tokens <= MAX_SAFE
                    && budget
                        .configured_trigger_tokens
                        .is_none_or(|value| value <= MAX_SAFE)
                    && budget.limits_for_scope(self.scope).is_ok()
            })
    }

    pub fn observed_tokens(&self) -> Option<u64> {
        self.actual_model_observed.then_some(())?;
        (self.model == self.actual_model).then_some(())?;
        self.input_tokens
            .zip(self.output_tokens)
            .and_then(|(input, output)| input.checked_add(output))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowUsageObservation {
    pub source_watermark: u64,
    pub rollback_epoch: u64,
    pub sample: WindowUsageSample,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowUsageStatus {
    Observed,
    Estimated,
    Unknown,
    Stale,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowUsageResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub model: ModelIdentity,
    pub window_id: Option<Uuid>,
    pub watermark: u64,
    pub rollback_epoch: u64,
    pub status: WindowUsageStatus,
    pub current_tokens: Option<u64>,
    pub observation: Option<WindowUsageObservation>,
}
