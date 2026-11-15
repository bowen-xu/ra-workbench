//! Existing prepared Agent transport, closed over the same original owners.
#[path = "docling_execution/workflow.rs"]
mod workflow;
pub(crate) use workflow::{
    DoclingHost, PreparedOverviewRun, PreparedOverviewRunState, run_docling_paper_overview,
};

#[path = "docling_execution/native.rs"]
mod native;
#[path = "docling_execution/terminal.rs"]
mod terminal;
pub(crate) use native::NativeDoclingHost;

use super::{
    CancellationSignal,
    error::AiError,
    generation_contracts::{PaperGenerationResponse, ProviderInput},
    reading_contracts::LlmModelSelection,
};
use crate::{
    agent::{
        model::{AgentHost, AgentModelResolver, AgentModelSelection, PreparedAgentModel},
        service::AgentSessionService,
    },
    storage::AppStore,
};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct DoclingExecution {
    store: AppStore,
    models: AgentModelResolver,
    sessions: Option<Arc<AgentSessionService>>,
}
impl DoclingExecution {
    pub(crate) fn new(
        store: AppStore,
        models: AgentModelResolver,
        sessions: Option<Arc<AgentSessionService>>,
    ) -> Self {
        Self {
            store,
            models,
            sessions,
        }
    }
    pub(crate) async fn prepare_model(
        &self,
        selection: &LlmModelSelection,
    ) -> Result<PreparedAgentModel, AiError> {
        self.models
            .prepare(
                AgentHost::Review,
                Some(AgentModelSelection {
                    provider_id: selection.provider_id.clone(),
                    model_id: selection.model_id.clone(),
                    reasoning_effort: selection.reasoning_effort.clone(),
                    service_tier: None,
                }),
            )
            .await
            .map_err(|_| AiError::Provider("invalid_request"))
    }
    pub(crate) async fn execute(
        &self,
        model: PreparedAgentModel,
        input: ProviderInput,
        shared: Option<(uuid::Uuid, uuid::Uuid)>,
        cancellation: Option<CancellationSignal>,
    ) -> Result<PaperGenerationResponse, AiError> {
        let has_images = input.has_images();
        let response =
            if let (Some(sessions), Some((conversation, request))) = (&self.sessions, shared) {
                if has_images {
                    super::paper_reading_shared_transport::call_prepared_vision_submission(
                        self.store.clone(),
                        sessions.clone(),
                        model,
                        input,
                        conversation,
                        request,
                        cancellation.unwrap_or_default(),
                    )
                    .await
                } else {
                    super::paper_reading_shared_transport::call_prepared_text_submission(
                        self.store.clone(),
                        sessions.clone(),
                        model,
                        input,
                        conversation,
                        request,
                        cancellation.unwrap_or_default(),
                    )
                    .await
                }
            } else if has_images {
                super::paper_reading_shared_transport::call_prepared_vision(
                    model,
                    input,
                    true,
                    None,
                    cancellation,
                )
                .await
            } else {
                super::paper_reading_shared_transport::call_prepared_text(
                    model,
                    input,
                    true,
                    8_000,
                    cancellation,
                )
                .await
            }?;
        Ok(PaperGenerationResponse {
            content: response.content,
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        })
    }
}
