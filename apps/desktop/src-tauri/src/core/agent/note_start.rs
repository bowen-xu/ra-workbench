//! Execute a confirmed saved-note snapshot with the application-owned runner.
use super::{
    composed_tools::ComposedToolExecutor,
    execution::AgentExecution,
    execution_profile::{AgentExecutionMode, AgentExecutionProfile, execution_profile},
    model::{AgentHost, AgentModelResolver, AgentModelSelection},
    note_submission::{
        self, Receipt,
        source::{Resource, SourceReceipt},
    },
    repository::SqliteSubmissionStore,
    service::{AgentSessionService, SubmissionAdmission},
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::ConversationId,
    model::{
        ModelCapabilities, ModelError, ModelFuture, ModelRequest, ModelStream, ModelTransport,
    },
    protocol::ModelIdentity,
    submission::SubmissionRecord,
};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub page_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub mode: AgentExecutionMode,
}
pub(super) struct SelectedStartRequest {
    pub receipt: SourceReceipt,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub mode: AgentExecutionMode,
    pub(super) call_limits: Option<(usize, usize)>,
}
impl From<StartRequest> for SelectedStartRequest {
    fn from(request: StartRequest) -> Self {
        Self {
            receipt: SourceReceipt {
                conversation_id: request.conversation_id,
                request_id: request.request_id,
                resource: Resource::Page(request.page_id),
            },
            context_snapshot_id: request.context_snapshot_id,
            model: request.model,
            mode: request.mode,
            call_limits: None,
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "reason")]
pub enum Error {
    UnsupportedModel,
    ConfirmationMismatch,
    Preparation(note_submission::Error),
    ModelUnavailable,
    RecoveryRequired,
    Queue(super::queue_contract::CommandError),
}

impl From<super::note_queue_admission::Error> for Error {
    fn from(error: super::note_queue_admission::Error) -> Self {
        match error {
            super::note_queue_admission::Error::ConfirmationMismatch => Self::ConfirmationMismatch,
            super::note_queue_admission::Error::Queue(error) => Self::Queue(error),
            super::note_queue_admission::Error::RecoveryRequired => Self::RecoveryRequired,
        }
    }
}

#[tauri::command]
pub(crate) async fn agent_start_note_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: StartRequest,
) -> Result<super::submission_response::StartResponse, Error> {
    let conversation = request.conversation_id;
    let request_id = request.request_id;
    let admitted = start(&store, &models, sessions.inner().clone(), request).await?;
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
    start_selected(store, models, sessions, request.into()).await
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
    start_selected(store, models, sessions, selected).await
}
pub(super) async fn start_selected(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: SelectedStartRequest,
) -> Result<SubmissionAdmission, Error> {
    start_selected_in_queue(store, models, sessions, request, None).await
}
pub(super) async fn start_selected_in_queue(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: SelectedStartRequest,
    queue: Option<super::queue_contract::Confirmation>,
) -> Result<SubmissionAdmission, Error> {
    start_selected_checked(
        store,
        sessions,
        request,
        queue,
        Some(models.clone()),
        |record, profile| async move {
            let prepared = models
                .prepare(
                    AgentHost::Notes,
                    Some(AgentModelSelection {
                        provider_id: record.input.model.provider_id.clone(),
                        model_id: record.input.model.model_id.clone(),
                        reasoning_effort: record.input.reasoning_effort.clone(),
                        service_tier: record.input.service_tier.clone(),
                    }),
                )
                .await
                .map_err(|_| Error::ModelUnavailable)?;
            profile
                .transport_with_model_history(
                    store,
                    &record,
                    prepared,
                    Some(models.clone()),
                    AgentHost::Notes,
                )
                .await
                .map_err(|_| Error::ModelUnavailable)
        },
    )
    .await
}

/// Only native code supplies the factory. Renderer input cannot choose tools,
/// prompts, transports or budgets. A repeated turn does not refresh credentials.
#[cfg(test)]
pub(crate) async fn start_with_factory<F, Fut>(
    store: &AppStore,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    factory: F,
) -> Result<SubmissionAdmission, Error>
where
    F: FnOnce(SubmissionRecord, AgentExecutionProfile) -> Fut,
    Fut: Future<Output = Result<Arc<dyn ModelTransport>, Error>>,
{
    start_selected_with_factory(store, sessions, request.into(), factory).await
}
#[cfg(test)]
pub(super) async fn start_selected_with_factory<F, Fut>(
    store: &AppStore,
    sessions: Arc<AgentSessionService>,
    request: SelectedStartRequest,
    factory: F,
) -> Result<SubmissionAdmission, Error>
where
    F: FnOnce(SubmissionRecord, AgentExecutionProfile) -> Fut,
    Fut: Future<Output = Result<Arc<dyn ModelTransport>, Error>>,
{
    start_selected_checked(store, sessions, request, None, None, factory).await
}

#[cfg(test)]
pub(super) async fn start_selected_with_tools_factory<F, Fut>(
    store: &AppStore,
    models: AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: SelectedStartRequest,
    factory: F,
) -> Result<SubmissionAdmission, Error>
where
    F: FnOnce(SubmissionRecord, AgentExecutionProfile) -> Fut,
    Fut: Future<Output = Result<Arc<dyn ModelTransport>, Error>>,
{
    start_selected_checked(store, sessions, request, None, Some(models), factory).await
}
pub(super) async fn start_selected_checked<F, Fut>(
    store: &AppStore,
    sessions: Arc<AgentSessionService>,
    request: SelectedStartRequest,
    queue: Option<super::queue_contract::Confirmation>,
    models: Option<AgentModelResolver>,
    factory: F,
) -> Result<SubmissionAdmission, Error>
where
    F: FnOnce(SubmissionRecord, AgentExecutionProfile) -> Fut,
    Fut: Future<Output = Result<Arc<dyn ModelTransport>, Error>>,
{
    let receipt = request.receipt;
    let record = note_submission::restore_selected(store, receipt)
        .await
        .map_err(Error::Preparation)?;
    if record.context_snapshot_id != request.context_snapshot_id
        || record.input.model != request.model
    {
        return Err(Error::ConfirmationMismatch);
    }
    let mut profile = execution_profile(&record.input.model, request.mode)
        .and_then(|profile| profile.with_submission_compaction(&record.input))
        .map_err(|_| Error::UnsupportedModel)?;
    if let Some((model_calls, tool_calls)) = request.call_limits {
        profile.limits = profile.limits.with_call_budget(model_calls, tool_calls);
        if profile.limits.model_calls == Some(0) {
            return Err(Error::UnsupportedModel);
        }
    }
    if record
        .input
        .reasoning_effort
        .as_ref()
        .is_some_and(|effort| !profile.capabilities.reasoning_efforts.contains(effort))
    {
        return Err(Error::UnsupportedModel);
    }
    super::note_queue_admission::check_origin(store, receipt, &record, queue).await?;
    let id = ConversationId::from_uuid(record.conversation_id);
    let history = sessions
        .open(id)
        .await
        .map_err(|_| Error::RecoveryRequired)?;
    if let Some(turn) = history.turn(record.turn_id) {
        if let Some(queue) = queue {
            super::note_queue_admission::admit(store, receipt, &record, queue).await?;
        }
        return Ok(SubmissionAdmission::AlreadyStarted {
            turn_id: record.turn_id,
            outcome: turn.outcome,
        });
    }
    let transport = factory(record.clone(), profile.clone()).await?;
    // Authentication may have awaited external work; no stale binding admission.
    if note_submission::restore_selected(store, receipt)
        .await
        .map_err(Error::Preparation)?
        != record
    {
        return Err(Error::ConfirmationMismatch);
    }
    if let Some(queue) = queue {
        super::note_queue_admission::admit(store, receipt, &record, queue).await?;
    }
    #[cfg(unix)]
    let mcp_runtime = super::child_runtime::mcp_runtime_for_conversation(
        store,
        &sessions,
        request.receipt.conversation_id,
    )
    .await
    .map(|runtime| {
        if transport.supports_namespace_tools() {
            runtime.namespaced_view()
        } else {
            runtime
        }
    });
    // Subtask spawning is disabled until its recovery path is complete.
    let (primary_tools, mut tool_definitions): (
        Arc<dyn research_agent_core::runner::ToolExecutor>,
        Vec<_>,
    ) = match receipt.resource {
        Resource::Page(page_id) => (
            Arc::new(super::note_tools::NoteTools {
                store: store.clone(),
                receipt: Receipt {
                    conversation_id: receipt.conversation_id,
                    request_id: receipt.request_id,
                    page_id,
                },
                record: record.clone(),
            }),
            super::note_tools::definitions(),
        ),
        Resource::Reading(_) => (Arc::new(ReadOnlyNoteTools), vec![]),
    };
    let skill_tools = super::skills::application::tools(store.clone(), record.clone())
        .await
        .map_err(|_| Error::RecoveryRequired)?;
    let skill_definitions = skill_tools.definitions();
    let primary_tools: Arc<dyn research_agent_core::runner::ToolExecutor> =
        Arc::new(ComposedToolExecutor::new(
            primary_tools,
            tool_definitions.clone(),
            Some(Arc::new(skill_tools)),
            skill_definitions.clone(),
        ));
    tool_definitions.extend(skill_definitions);
    #[cfg(unix)]
    let mcp_definitions = mcp_runtime
        .as_ref()
        .map(|runtime| runtime.definitions())
        .unwrap_or_default();
    #[cfg(not(unix))]
    let mcp_definitions = Vec::new();
    tool_definitions.extend(mcp_definitions.clone());
    let tools: Arc<dyn research_agent_core::runner::ToolExecutor> =
        Arc::new(ComposedToolExecutor::new(
            primary_tools,
            tool_definitions
                .iter()
                .filter(|definition| {
                    !mcp_definitions.iter().any(|mcp| {
                        mcp.name == definition.name && mcp.namespace == definition.namespace
                    })
                })
                .cloned(),
            #[cfg(unix)]
            mcp_runtime
                .map(|runtime| runtime as Arc<dyn research_agent_core::runner::ToolExecutor>),
            mcp_definitions.clone(),
        ));
    let registered = super::host_execution::registration_with_hooks(
        store.clone(),
        record.clone(),
        sessions.clone(),
        research_agent_core::tool_registry::OwnedToolRegistration::new(tool_definitions, tools),
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
        stop_hooks: match &models {
            Some(models) => super::execution::NativeStopBinding::prepare(
                models,
                AgentHost::Notes,
                store,
                &record,
                registered.workspace.clone(),
            )
            .await
            .map_err(|_| Error::RecoveryRequired)?,
            None => None,
        },
        compaction_hooks: match &models {
            Some(models) => models
                .prepare_compaction_hooks_for_profile(
                    AgentHost::Notes,
                    &record,
                    &profile,
                    registered.workspace,
                )
                .await
                .map_err(|_| Error::RecoveryRequired)?,
            None => None,
        },
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
        system_instructions: crate::agent::host_instructions::NOTES.into(),
        limits: profile.limits,
        max_output_tokens: profile.max_output_tokens,
        max_input_bytes: profile.max_input_bytes,
    });
    execution
        .start_with_steering(
            sessions,
            Arc::new(SqliteSubmissionStore::for_app(store)),
            id,
            record.request_id,
            Some(Arc::new(
                super::repository::steering::SqliteSteeringStore::new(
                    store.shared_agent_connection(),
                ),
            )),
        )
        .await
        .map_err(|_| Error::RecoveryRequired)
}

/// Check again at every model dispatch, including the first dispatch after
/// durable admission. Binding removal blocks subsequent disclosure, without
/// pretending to retract bytes already sent by an in-flight request.
struct BoundTransport {
    store: AppStore,
    receipt: SourceReceipt,
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
            let saved = note_submission::restore_selected(&self.store, self.receipt)
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

/// Frozen-source read/search tools are composed by AgentExecution. Reading notes
/// do not acquire the independent Note Page rewrite tool or ambient execution.
struct ReadOnlyNoteTools;
impl research_agent_core::runner::ToolExecutor for ReadOnlyNoteTools {
    fn validate(
        &self,
        _: &research_agent_core::model::ToolCall,
    ) -> Result<(), research_agent_core::runner::RunError> {
        Err(research_agent_core::runner::RunError::ToolRejected)
    }
    fn authorize_and_execute(
        &self,
        _: research_agent_core::model::ToolCall,
    ) -> research_agent_core::runner::RunFuture<'_, research_agent_core::runner::ToolOutput> {
        Box::pin(async { Err(research_agent_core::runner::RunError::ToolRejected) })
    }
}
