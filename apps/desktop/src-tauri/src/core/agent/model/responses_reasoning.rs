//! Fixed ModelClient::build_reasoning, e363b08c9175ac1cbe5893615dd2cb9ddf95043b.
//! Apache-2.0 / Copyright 2025 OpenAI. Native composition resolves support and
//! freezes the selection for an exact model; absence means omit, never infer.
use super::responses_transport::ResponsesTransport;
use research_agent_core::{model::ModelError, protocol::ModelIdentity};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesReasoningSummary {
    Auto,
    Concise,
    Detailed,
}

impl ResponsesTransport {
    /// Caller has verified this exact model's summary capability. This value is
    /// provider-owned policy, not a renderer request or a model-name heuristic.
    pub fn with_reasoning_summary(
        mut self,
        model: ModelIdentity,
        summary: Option<ResponsesReasoningSummary>,
    ) -> Result<Self, ModelError> {
        if model.provider_id != self.config.provider_id
            || model.model_id.trim().is_empty()
            || model
                .revision
                .as_ref()
                .is_some_and(|revision| revision.trim().is_empty())
        {
            return Err(ModelError::InvalidRequest);
        }
        self.reasoning_summary = summary.map(|summary| (model, summary));
        Ok(self)
    }

    pub(super) fn apply_reasoning_summary(
        &self,
        model: &ModelIdentity,
        body: &mut Value,
    ) -> Result<(), ModelError> {
        if let Some((expected, summary)) = &self.reasoning_summary {
            if expected != model {
                return Err(ModelError::UnsupportedCapability);
            }
            if body.get("reasoning").is_none() {
                body["reasoning"] = json!({});
            }
            body["reasoning"]["summary"] = json!(summary);
        }
        Ok(())
    }
}
