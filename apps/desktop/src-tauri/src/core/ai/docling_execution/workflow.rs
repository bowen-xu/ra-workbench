//! Original Docling sequence shared by static and selected Native composition.
use crate::ai::{
    candidate_contracts::AiEvidenceBundle,
    error::{AiCommandError, AiError},
    generation_support::{AiRun, PaperGenerationResponse, ProviderInput},
    paper_analysis_contracts::DoclingPaperOverviewDraft,
    reading_contracts::LlmModelSelection,
};
use std::time::Instant;

pub(crate) struct PreparedOverviewRun {
    pub(crate) state: PreparedOverviewRunState,
    pub(crate) selection: LlmModelSelection,
    pub(crate) input: ProviderInput,
}
pub(crate) struct PreparedOverviewRunState {
    pub(crate) run: AiRun,
    pub(crate) generation_stage_id: String,
    pub(crate) generation_started: Instant,
    pub(crate) bundle: AiEvidenceBundle,
}

pub(crate) trait DoclingHost: Sync {
    fn prepare(
        &self,
        draft: DoclingPaperOverviewDraft,
    ) -> impl Future<Output = Result<PreparedOverviewRun, AiCommandError>> + Send;
    fn prepare_model(
        &self,
        selection: &LlmModelSelection,
    ) -> impl Future<Output = Result<crate::agent::model::PreparedAgentModel, AiError>> + Send;
    fn execute(
        &self,
        model: crate::agent::model::PreparedAgentModel,
        input: ProviderInput,
        shared: Option<(uuid::Uuid, uuid::Uuid)>,
        cancellation: Option<crate::ai::CancellationSignal>,
    ) -> impl Future<Output = Result<PaperGenerationResponse, AiError>> + Send;
    fn success(
        &self,
        state: PreparedOverviewRunState,
        response: PaperGenerationResponse,
    ) -> impl Future<Output = Result<AiRun, AiCommandError>> + Send;
    fn failure(
        &self,
        state: PreparedOverviewRunState,
        error: AiError,
    ) -> impl Future<Output = Result<AiRun, AiCommandError>> + Send;
}

pub(crate) async fn run_docling_paper_overview(
    host: &impl DoclingHost,
    draft: DoclingPaperOverviewDraft,
    cancellation: Option<crate::ai::CancellationSignal>,
) -> Result<AiRun, AiCommandError> {
    let shared_conversation = draft.shared_conversation_id;
    let shared_request = draft.shared_request_id;
    let PreparedOverviewRun {
        state,
        selection,
        input,
    } = host.prepare(draft).await?;
    let conversation = shared_conversation.or_else(|| {
        state
            .run
            .shared_conversation_id
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
    });
    let request = shared_request.or_else(|| {
        state
            .run
            .shared_request_id
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
    });
    let model = host.prepare_model(&selection).await?;
    match host
        .execute(model, input, conversation.zip(request), cancellation)
        .await
    {
        Ok(response) => host.success(state, response).await,
        Err(error) => host.failure(state, error).await,
    }
}
