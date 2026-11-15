//! Explicit confirmation of a frozen reading-note submission on the shared runner.
use super::{
    execution_profile::AgentExecutionMode,
    model::AgentModelResolver,
    note_start::{self, Error, SelectedStartRequest},
    note_submission::source::{Resource, SourceReceipt},
    service::{AgentSessionService, SubmissionAdmission},
};
use crate::storage::AppStore;
use research_agent_core::protocol::ModelIdentity;
use serde::Deserialize;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub reading_note_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub mode: AgentExecutionMode,
}
impl From<StartRequest> for SelectedStartRequest {
    fn from(request: StartRequest) -> Self {
        Self {
            receipt: SourceReceipt {
                conversation_id: request.conversation_id,
                request_id: request.request_id,
                resource: Resource::Reading(request.reading_note_id),
            },
            context_snapshot_id: request.context_snapshot_id,
            model: request.model,
            mode: request.mode,
            call_limits: None,
        }
    }
}
pub(crate) async fn start(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
) -> Result<SubmissionAdmission, Error> {
    note_start::start_selected(store, models, sessions, request.into()).await
}

pub(crate) async fn start_bounded(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    model_calls: usize,
    tool_calls: usize,
) -> Result<SubmissionAdmission, Error> {
    let mut selected: SelectedStartRequest = request.into();
    selected.call_limits = Some((model_calls, tool_calls));
    note_start::start_selected(store, models, sessions, selected).await
}
#[tauri::command]
pub(crate) async fn agent_start_reading_note_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: StartRequest,
) -> Result<super::submission_response::StartResponse, Error> {
    let conversation = request.conversation_id;
    let request_id = request.request_id;
    Ok(super::submission_response::StartResponse::from_admission(
        conversation,
        request_id,
        start(&store, &models, sessions.inner().clone(), request).await?,
    ))
}
