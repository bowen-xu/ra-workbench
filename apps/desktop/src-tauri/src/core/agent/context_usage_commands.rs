//! Read-only window status; no renderer thresholds or model/credential preparation.
use super::{
    context_usage::WindowUsageResponse, service::AgentSessionService,
    session_commands::SessionCommandError,
};
use research_agent_core::protocol::ModelIdentity;
use serde::Deserialize;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowUsageRequest {
    pub conversation_id: Uuid,
    pub model: ModelIdentity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_query_does_not_accept_execution_or_threshold_fields() {
        let request = serde_json::json!({
            "conversationId": Uuid::new_v4(),
            "model": { "providerId": "fixture", "modelId": "model", "revision": null }
        });
        assert!(serde_json::from_value::<WindowUsageRequest>(request.clone()).is_ok());
        for field in ["threshold", "prompt", "credentials", "windowId"] {
            let mut invalid = request.clone();
            invalid[field] = serde_json::json!(1);
            assert!(serde_json::from_value::<WindowUsageRequest>(invalid).is_err());
        }
    }
}

#[tauri::command]
pub async fn agent_read_context_usage(
    service: State<'_, Arc<AgentSessionService>>,
    request: WindowUsageRequest,
) -> Result<WindowUsageResponse, SessionCommandError> {
    service
        .context_usage(request.conversation_id, request.model)
        .await
        .map_err(Into::into)
}
