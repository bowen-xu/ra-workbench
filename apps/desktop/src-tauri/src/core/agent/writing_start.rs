//! Explicit execution admission for a previously reviewed writing snapshot.
use super::{
    execution_profile::execution_profile,
    model::AgentModelResolver,
    service::AgentSessionService,
    writing_commands::PreviewError,
    writing_submission::{
        WritingExecutionConfirmation, WritingStartError, WritingSubmissionReceipt,
        WritingSubmissionService,
    },
};
use research_agent_core::protocol::ModelIdentity;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

pub use super::execution_profile::AgentExecutionMode as WritingExecutionMode;
#[path = "writing_start/queued_status.rs"]
mod queued_status;
#[path = "writing_start/status.rs"]
mod status;
pub use queued_status::QueuedStartStatusResponse;
pub(crate) use queued_status::read_status;
pub use status::StartStatusResponse;
pub(crate) use status::{read_compaction_start_status, read_start_status};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub manuscript_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub mode: WritingExecutionMode,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueuedStartRequest {
    pub input_id: Uuid,
    pub expected_revision: u64,
    pub start: StartRequest,
}
pub use super::submission_response::StartResponse;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "reason")]
pub enum StartError {
    UnsupportedModel,
    ConfirmationMismatch,
    Preparation(PreviewError),
    Unavailable,
    RecoveryRequired,
}

#[tauri::command]
pub async fn agent_start_writing_submission(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: StartRequest,
) -> Result<StartResponse, StartError> {
    start(&service, &models, sessions.inner().clone(), request).await
}

#[tauri::command]
pub async fn agent_read_writing_start_status(
    service: State<'_, WritingSubmissionService>,
    request: StartRequest,
) -> Result<StartStatusResponse<StartRequest>, StartError> {
    read_start_status(&service, request).await
}

pub(crate) async fn start(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
) -> Result<StartResponse, StartError> {
    start_impl(service, models, sessions, request, None).await
}

#[tauri::command]
pub async fn agent_start_writing_queued_submission(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: QueuedStartRequest,
) -> Result<StartResponse, StartError> {
    start_queued(&service, &models, sessions.inner().clone(), request).await
}

#[tauri::command]
pub async fn agent_read_writing_queued_start_status(
    service: State<'_, WritingSubmissionService>,
    request: QueuedStartRequest,
) -> Result<QueuedStartStatusResponse, StartError> {
    read_status(&service, request).await
}

pub(crate) async fn start_queued(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: QueuedStartRequest,
) -> Result<StartResponse, StartError> {
    if request.input_id.is_nil() || request.expected_revision >= 9_007_199_254_740_991 {
        return Err(StartError::ConfirmationMismatch);
    }
    start_impl(
        service,
        models,
        sessions,
        request.start,
        Some((request.input_id, request.expected_revision)),
    )
    .await
}

async fn start_impl(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    queue: Option<(Uuid, u64)>,
) -> Result<StartResponse, StartError> {
    let profile = execution_profile(&request.model, request.mode)
        .map_err(|_| StartError::UnsupportedModel)?;
    let receipt = WritingSubmissionReceipt {
        conversation_id: request.conversation_id,
        request_id: request.request_id,
        manuscript_id: request.manuscript_id,
    };
    let prepared = service
        .restore(models, receipt)
        .await
        .map_err(|e| StartError::Preparation(e.into()))?;
    let profile = profile
        .with_submission_compaction(&prepared.record.input)
        .map_err(|_| StartError::UnsupportedModel)?;
    if prepared
        .record
        .input
        .reasoning_effort
        .as_ref()
        .is_some_and(|effort| !profile.capabilities.reasoning_efforts.contains(effort))
    {
        return Err(StartError::UnsupportedModel);
    }
    let confirmation = WritingExecutionConfirmation {
        receipt,
        context_snapshot_id: request.context_snapshot_id,
        model: request.model,
    };
    let admitted = if let Some(queue) = queue {
        service
            .start_queued_confirmed(sessions, prepared, confirmation, profile, queue)
            .await
    } else {
        service
            .start_confirmed(sessions, prepared, confirmation, profile)
            .await
    }
    .map_err(|error| match error {
        WritingStartError::Mismatch => StartError::ConfirmationMismatch,
        WritingStartError::Preparation(error) => StartError::Preparation(error.into()),
        WritingStartError::Run(_) => StartError::RecoveryRequired,
        WritingStartError::Model(_) => StartError::Unavailable,
    })?;
    Ok(StartResponse::from_admission(
        request.conversation_id,
        request.request_id,
        admitted,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_start_does_not_accept_execution_overrides() {
        let base = serde_json::json!({"inputId":Uuid::new_v4(),"expectedRevision":2,"start":{
            "conversationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),
            "contextSnapshotId":Uuid::new_v4(),"model":{"providerId":"deepseek","modelId":"deepseek-v4-pro","revision":null},"mode":"deepSeekNonThinking"
        }});
        assert!(serde_json::from_value::<QueuedStartRequest>(base.clone()).is_ok());
        let mut invalid = base;
        invalid["text"] = serde_json::json!("override");
        assert!(serde_json::from_value::<QueuedStartRequest>(invalid).is_err());
    }
}
