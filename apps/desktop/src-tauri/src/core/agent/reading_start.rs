//! Execute a confirmed paper submission through the shared Agent runner.
//! Image materialization is bound to the same immutable snapshot and submission
//! receipt; the UI/model matrix still needs real-provider acceptance.
use super::{
    composed_tools::RejectingToolExecutor,
    execution::AgentExecution,
    execution_profile::{AgentExecutionMode, execution_profile_for_host},
    model::{AgentHost, AgentModelResolver, AgentModelSelection},
    reading_submission::{self, Error as SubmissionError, Receipt},
    repository::{AppImageMaterializer, SqliteSubmissionStore},
    service::{AgentSessionService, SubmissionAdmission},
};
use crate::storage::AppStore;
use research_agent_core::{
    model::{
        ModelCapabilities, ModelError, ModelFuture, ModelRequest, ModelStream, ModelTransport,
        ToolDefinition,
    },
    protocol::ModelIdentity,
    runner::ToolExecutor,
    submission::SubmissionRecord,
    tool_registry::OwnedToolRegistration,
};
use serde::Deserialize;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;
#[path = "reading_start/status.rs"]
pub(crate) mod status;

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub paper_id: String,
    pub paper_context_snapshot_id: String,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub mode: AgentExecutionMode,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "reason")]
pub enum Error {
    UnsupportedModel,
    ConfirmationMismatch,
    Preparation(SubmissionError),
    ModelUnavailable,
    RecoveryRequired,
}

#[tauri::command]
pub(crate) async fn agent_start_paper_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: StartRequest,
    expected_epoch: Option<Uuid>,
) -> Result<super::submission_response::StartResponse, Error> {
    let conversation = request.conversation_id;
    let request_id = request.request_id;
    let admitted = if expected_epoch.is_none() {
        start(&store, &models, sessions.inner().clone(), request).await?
    } else {
        start_with_epoch(
            &store,
            &models,
            sessions.inner().clone(),
            request,
            expected_epoch,
        )
        .await?
    };
    Ok(super::submission_response::StartResponse::from_admission(
        conversation,
        request_id,
        admitted,
    ))
}

pub(crate) async fn start(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
) -> Result<SubmissionAdmission, Error> {
    start_with_call_limits(store, models, sessions, request, None, None).await
}

pub(crate) async fn start_with_epoch(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    expected_epoch: Option<Uuid>,
) -> Result<SubmissionAdmission, Error> {
    start_with_call_limits(store, models, sessions, request, None, expected_epoch).await
}

pub(crate) async fn start_bounded(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    model_calls: usize,
    tool_calls: usize,
) -> Result<SubmissionAdmission, Error> {
    start_with_call_limits(
        store,
        models,
        sessions,
        request,
        Some((model_calls, tool_calls)),
        None,
    )
    .await
}

async fn start_with_call_limits(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    call_limits: Option<(usize, usize)>,
    expected_epoch: Option<Uuid>,
) -> Result<SubmissionAdmission, Error> {
    if store.native_note_trace().is_none() {
        store
            .require_paper_reading_source()
            .map_err(|_| Error::Preparation(reading_submission::Error::Unavailable))?;
    }
    if let Some(epoch) = expected_epoch {
        let database = store.shared_agent_connection();
        if !sessions.owns_data(&database) {
            return Err(Error::RecoveryRequired);
        }
        let operation = request.request_id;
        tokio::task::spawn_blocking(move || {
            database.check_catalog_epoch(operation, epoch, "paper-start-epoch")
        })
        .await
        .map_err(|_| Error::RecoveryRequired)?
        .map_err(|error| Error::Preparation(reading_submission::storage_error(error)))?;
    }
    let mut profile = execution_profile_for_host(&request.model, request.mode, AgentHost::Reading)
        .map_err(|_| Error::UnsupportedModel)?;
    if let Some((model_calls, tool_calls)) = call_limits {
        profile.limits = profile.limits.with_call_budget(model_calls, tool_calls);
        if profile.limits.model_calls == Some(0) {
            return Err(Error::UnsupportedModel);
        }
    }
    let receipt = Receipt {
        conversation_id: request.conversation_id,
        request_id: request.request_id,
        paper_id: request.paper_id,
        paper_context_snapshot_id: request.paper_context_snapshot_id,
    };
    let record = reading_submission::restore(store, &receipt)
        .await
        .map_err(Error::Preparation)?;
    if record.context_snapshot_id != request.context_snapshot_id
        || record.input.model != request.model
    {
        return Err(Error::ConfirmationMismatch);
    }
    profile = profile
        .with_submission_compaction(&record.input)
        .map_err(|_| Error::UnsupportedModel)?;
    if record
        .input
        .reasoning_effort
        .as_ref()
        .is_some_and(|effort| !profile.capabilities.reasoning_efforts.contains(effort))
    {
        return Err(Error::UnsupportedModel);
    }
    let prepared = models
        .prepare(
            AgentHost::Reading,
            Some(AgentModelSelection {
                provider_id: record.input.model.provider_id.clone(),
                model_id: record.input.model.model_id.clone(),
                reasoning_effort: record.input.reasoning_effort.clone(),
                service_tier: record.input.service_tier.clone(),
            }),
        )
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    let transport = profile
        .transport_with_model_history(
            store,
            &record,
            prepared,
            Some(models.clone()),
            AgentHost::Reading,
        )
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    #[cfg(unix)]
    let mcp_runtime = super::child_runtime::mcp_runtime_for_conversation(
        store,
        &sessions,
        request.conversation_id,
    )
    .await
    .map(|runtime| {
        if transport.supports_namespace_tools() {
            runtime.namespaced_view()
        } else {
            runtime
        }
    });
    #[cfg(unix)]
    let fallback = mcp_runtime
        .clone()
        .map(|runtime| runtime as Arc<dyn ToolExecutor>)
        .unwrap_or_else(|| Arc::new(RejectingToolExecutor));
    #[cfg(not(unix))]
    let fallback = Arc::new(RejectingTools) as Arc<dyn ToolExecutor>;
    let skill_tools = super::skills::application::tools(store.clone(), record.clone())
        .await
        .map_err(|_| Error::RecoveryRequired)?;
    let skill_definitions = skill_tools.definitions();
    // AgentExecution owns the built-in frozen-source definitions for every
    // host and wraps this reading-specific executor with SubmissionTools.
    // Keep this list limited to reading extensions so schema and execution
    // have the same single owner.
    #[cfg(unix)]
    let mcp_definitions = mcp_runtime
        .as_ref()
        .map(|runtime| runtime.definitions())
        .unwrap_or_default();
    #[cfg(not(unix))]
    let mcp_definitions = Vec::new();
    let host_tools = compose_host_tools(
        Arc::new(super::composed_tools::RejectingToolExecutor),
        Arc::new(skill_tools),
        skill_definitions,
        fallback,
        mcp_definitions,
    );
    let registered = super::host_execution::registration_with_hooks(
        store.clone(),
        record.clone(),
        sessions.clone(),
        host_tools,
    )
    .await
    .map_err(|_| Error::RecoveryRequired)?;
    #[cfg(target_os = "macos")]
    let host_tools = super::compaction_hook_spill::tool::registration(
        store.clone(),
        record.clone(),
        &profile,
        registered.workspace.clone(),
        registered.tools,
    )
    .map_err(|_| Error::RecoveryRequired)?;
    #[cfg(not(target_os = "macos"))]
    let host_tools = registered.tools;
    let host_tools = super::context_session::resource_tools::registration(
        store.clone(),
        record.clone(),
        host_tools,
    );
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: super::execution::NativeStopBinding::prepare(
            models,
            AgentHost::Reading,
            store,
            &record,
            registered.workspace.clone(),
        )
        .await
        .map_err(|_| Error::RecoveryRequired)?,
        compaction_hooks: models
            .prepare_compaction_hooks_for_profile(
                AgentHost::Reading,
                &record,
                &profile,
                registered.workspace,
            )
            .await
            .map_err(|_| Error::RecoveryRequired)?,
        compaction_strategy: Some(
            profile
                .compaction_strategy(transport.clone())
                .map_err(|_| Error::RecoveryRequired)?,
        ),
        model: record.input.model.clone(),
        transport: Arc::new(BoundTransport {
            store: store.clone(),
            receipt,
            record: record.clone(),
            inner: transport,
        }),
        host_tools,
        system_instructions: crate::agent::host_instructions::READING.into(),
        limits: profile.limits,
        max_output_tokens: profile.max_output_tokens,
        max_input_bytes: profile.max_input_bytes,
    });
    execution
        .start_with_images_and_steering(
            sessions,
            Arc::new(SqliteSubmissionStore::for_app(store)),
            Arc::new(AppImageMaterializer::new(store)),
            research_agent_core::conversation::ConversationId::from_uuid(request.conversation_id),
            request.request_id,
            Some(Arc::new(
                super::repository::steering::SqliteSteeringStore::new(
                    store.shared_agent_connection(),
                ),
            )),
        )
        .await
        .map_err(|_| Error::RecoveryRequired)
}

pub(crate) fn reading_host_tool_definitions() -> Vec<ToolDefinition> {
    vec![]
}

fn compose_host_tools(
    child: Arc<dyn ToolExecutor>,
    skills: Arc<dyn ToolExecutor>,
    skill_definitions: Vec<ToolDefinition>,
    mcp: Arc<dyn ToolExecutor>,
    mcp_definitions: Vec<ToolDefinition>,
) -> OwnedToolRegistration {
    use super::composed_tools::ComposedToolExecutor;
    let mut primary_definitions = reading_host_tool_definitions();
    let primary = Arc::new(ComposedToolExecutor::new(
        child,
        primary_definitions.clone(),
        Some(skills),
        skill_definitions.clone(),
    ));
    primary_definitions.extend(skill_definitions);
    let combined = Arc::new(ComposedToolExecutor::new(
        primary,
        primary_definitions.clone(),
        Some(mcp),
        mcp_definitions.clone(),
    ));
    primary_definitions.extend(mcp_definitions);
    OwnedToolRegistration::new(primary_definitions, combined)
}

struct BoundTransport {
    store: AppStore,
    receipt: Receipt,
    record: SubmissionRecord,
    inner: Arc<dyn ModelTransport>,
}
impl ModelTransport for BoundTransport {
    fn tool_output_policy(
        &self,
    ) -> Option<research_agent_core::model::tool_output::ToolOutputPolicy> {
        self.inner.tool_output_policy()
    }
    fn pre_turn_compaction<'a>(
        &'a self,
        turn: uuid::Uuid,
        model: &'a research_agent_core::protocol::ModelIdentity,
        active: Option<u64>,
    ) -> ModelFuture<
        'a,
        Option<Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy>>,
    > {
        self.inner.pre_turn_compaction(turn, model, active)
    }

    fn prepare_context_policy(&self) -> ModelFuture<'_, ()> {
        self.inner.prepare_context_policy()
    }
    fn prepared_compaction_strategy(
        &self,
    ) -> Option<Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy>>
    {
        self.inner.prepared_compaction_strategy()
    }
    fn supports_compaction_kind(
        &self,
        kind: research_agent_core::context::compaction_strategy::CompactionKind,
    ) -> bool {
        self.inner.supports_compaction_kind(kind)
    }
    fn validate_compaction_window<'a>(
        &'a self,
        checkpoint: &'a research_agent_core::context::checkpoint::ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            self.authorize_request(request).await?;
            self.inner
                .validate_compaction_window(checkpoint, request)
                .await
        })
    }
    fn context_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        self.inner.context_accounting()
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn supports_namespace_tools(&self) -> bool {
        self.inner.supports_namespace_tools()
    }
    fn authorize_request<'a>(&'a self, request: &'a ModelRequest) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            let saved = reading_submission::restore(&self.store, &self.receipt)
                .await
                .map_err(|_| ModelError::Unavailable)?;
            if saved != self.record || request.model != self.record.input.model {
                return Err(ModelError::InvalidRequest);
            }
            Ok(())
        })
    }
    fn start_text_compaction(
        &self,
        request: ModelRequest,
    ) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.authorize_request(&request).await?;
            self.inner.start_text_compaction(request).await
        })
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.authorize_request(&request).await?;
            self.inner.start(request).await
        })
    }
}

#[cfg(test)]
#[path = "reading_start/tests.rs"]
mod tests;
