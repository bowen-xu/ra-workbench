//! Queue provenance is retained even after cancellation; ordinary start cannot bypass it.
use super::{
    execution_profile::AgentExecutionMode,
    model::AgentModelResolver,
    note_queue::Scope,
    note_start::{self, Error, SelectedStartRequest},
    note_submission::source::SourceReceipt,
    service::AgentSessionService,
    submission_response::StartResponse,
};
use crate::storage::AppStore;
use research_agent_core::protocol::ModelIdentity;
use serde::Deserialize;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

pub use super::queue_contract::Confirmation;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub scope: Scope,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub mode: AgentExecutionMode,
    pub queue: Confirmation,
}
#[tauri::command]
pub(crate) async fn agent_start_note_queued_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: StartRequest,
) -> Result<StartResponse, Error> {
    start(&store, &models, sessions.inner().clone(), request).await
}
pub(crate) async fn start(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
) -> Result<StartResponse, Error> {
    let receipt = SourceReceipt {
        conversation_id: request.conversation_id,
        request_id: request.request_id,
        resource: request.scope.resource(),
    };
    let admitted = note_start::start_selected_in_queue(
        store,
        models,
        sessions,
        SelectedStartRequest {
            receipt,
            context_snapshot_id: request.context_snapshot_id,
            model: request.model,
            mode: request.mode,
            call_limits: None,
        },
        Some(request.queue),
    )
    .await?;
    Ok(StartResponse::from_admission(
        receipt.conversation_id,
        receipt.request_id,
        admitted,
    ))
}
