//! Confirmed native /compact admission. Host adapters retain source authority;
//! the existing session/task owner retains execution, cancellation and retries.
use super::{
    execution::{AgentExecution, CompactionStart},
    execution_profile::{
        AgentExecutionProfile, execution_mode_for_model, execution_profile_for_host,
    },
    model::{AgentHost, AgentModelResolver, AgentModelSelection},
    repository::{AppImageMaterializer, SqliteSubmissionStore, WorkerRetention},
    service::{AgentSessionService, SubmissionAdmission, SubmissionRunError},
    session_commands::SessionCommandError,
    submission_response::StartResponse,
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::ConversationId,
    model::{ModelError, ModelFuture, ModelRequest, ModelStream, ModelTransport},
    protocol::ModelIdentity,
    submission::SubmissionRecord,
    tool_registry::OwnedToolRegistration,
};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc};
use tauri::State;
use uuid::Uuid;
#[path = "compaction_commands/catalog.rs"]
mod catalog;
#[path = "compaction_commands/preparation.rs"]
pub(crate) mod preparation;

#[derive(Clone, Deserialize, Serialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Host {
    Manuscript {
        manuscript_id: Uuid,
    },
    Paper {
        paper_id: String,
        paper_context_snapshot_id: String,
    },
    Note {
        page_id: Uuid,
    },
    ReadingNote {
        reading_note_id: Uuid,
    },
}
impl Host {
    fn instructions(&self) -> &'static str {
        match self {
            Self::Manuscript { .. } => super::host_instructions::WRITING,
            Self::Paper { .. } => super::host_instructions::READING,
            Self::Note { .. } | Self::ReadingNote { .. } => super::host_instructions::NOTES,
        }
    }
    fn model_host(&self) -> AgentHost {
        match self {
            Self::Manuscript { .. } => AgentHost::Writing,
            Self::Paper { .. } => AgentHost::Reading,
            Self::Note { .. } | Self::ReadingNote { .. } => AgentHost::Notes,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub expected_revision: u64,
    pub model: ModelIdentity,
    pub host: Host,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "reason")]
pub enum Error {
    InvalidInput,
    HostUnavailable,
    ConfirmationMismatch,
    UnsupportedModel,
    ModelUnavailable,
    HistoryChanged { expected: u64, actual: u64 },
    Session(SessionCommandError),
    Busy,
    RecoveryRequired,
}
impl From<SubmissionRunError> for Error {
    fn from(error: SubmissionRunError) -> Self {
        use super::tasks::TaskRegistryError;
        match error {
            SubmissionRunError::HistoryChanged { expected, actual } => {
                Self::HistoryChanged { expected, actual }
            }
            SubmissionRunError::Session(error) => Self::Session(error.into()),
            SubmissionRunError::Input(
                research_agent_core::submission::SubmissionError::Conflict,
            ) => Self::ConfirmationMismatch,
            SubmissionRunError::Task(TaskRegistryError::Active | TaskRegistryError::Capacity) => {
                Self::Busy
            }
            _ => Self::RecoveryRequired,
        }
    }
}
#[tauri::command]
pub(crate) async fn agent_start_compaction(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: StartRequest,
) -> Result<StartResponse, Error> {
    execute(
        store.inner(),
        models.inner(),
        sessions.inner().clone(),
        request,
    )
    .await
}

#[tauri::command]
pub(crate) async fn agent_read_writing_compaction_start_status(
    service: State<'_, super::writing_submission::WritingSubmissionService>,
    request: StartRequest,
) -> Result<super::writing_start::StartStatusResponse<StartRequest>, Error> {
    super::writing_start::read_compaction_start_status(&service, request).await
}

/// Tauri and the isolated native preview share the same prepared admission.
pub(crate) async fn execute(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
) -> Result<StartResponse, Error> {
    execute_retaining(store, models, sessions, request, WorkerRetention::default()).await
}

pub(crate) async fn execute_retaining(
    store: &AppStore,
    models: &AgentModelResolver,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    worker_retention: WorkerRetention,
) -> Result<StartResponse, Error> {
    let conversation_id = request.conversation_id;
    let request_id = request.request_id;
    let host = request.host.model_host();
    let catalog_host = request.host.clone();
    let transport_store = store.clone();
    let hook_store = store.clone();
    let hook_sessions = sessions.clone();
    let admitted = start_prepared_retaining(
        store,
        sessions,
        request,
        worker_retention,
        |record, profile| async move {
            let prepared = models
                .prepare(
                    host,
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
                    &transport_store,
                    &record,
                    prepared,
                    Some(models.clone()),
                    host,
                )
                .await
                .map_err(|_| Error::ModelUnavailable)?;
            let registered = catalog::prepare(
                hook_store,
                hook_sessions,
                record.clone(),
                models.clone(),
                &catalog_host,
                &profile,
                transport.supports_namespace_tools(),
            )
            .await?;
            let hooks = models
                .prepare_compaction_hooks_for_profile(host, &record, &profile, registered.workspace)
                .await
                .map_err(|_| Error::ModelUnavailable)?;
            Ok((transport, hooks, registered.tools))
        },
    )
    .await?;
    Ok(StartResponse::from_admission(
        conversation_id,
        request_id,
        admitted,
    ))
}

async fn restore(store: &AppStore, request: &StartRequest) -> Result<SubmissionRecord, Error> {
    let conversation_id = request.conversation_id;
    let request_id = request.request_id;
    match &request.host {
        Host::Manuscript { manuscript_id } => {
            super::writing_submission::WritingSubmissionService::from_app_store(store)
                .restore_record(super::writing_submission::WritingSubmissionReceipt {
                    conversation_id,
                    request_id,
                    manuscript_id: *manuscript_id,
                })
                .await
                .map_err(|_| Error::HostUnavailable)
        }
        Host::Paper {
            paper_id,
            paper_context_snapshot_id,
        } => super::reading_submission::restore(
            store,
            &super::reading_submission::Receipt {
                conversation_id,
                request_id,
                paper_id: paper_id.clone(),
                paper_context_snapshot_id: paper_context_snapshot_id.clone(),
            },
        )
        .await
        .map_err(|_| Error::HostUnavailable),
        Host::Note { page_id } => super::note_submission::restore(
            store,
            super::note_submission::Receipt {
                conversation_id,
                request_id,
                page_id: *page_id,
            },
        )
        .await
        .map_err(|_| Error::HostUnavailable),
        Host::ReadingNote { reading_note_id } => super::reading_note_submission::restore(
            store,
            super::reading_note_submission::Receipt {
                conversation_id,
                request_id,
                reading_note_id: *reading_note_id,
            },
        )
        .await
        .map_err(|_| Error::HostUnavailable),
    }
}

/// Only native code injects the transport factory. No renderer-supplied tools,
/// prompts or budgets are accepted. Existing receipts never refresh credentials.
#[cfg(test)]
async fn start_checked<F, Fut>(
    store: &AppStore,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    factory: F,
) -> Result<SubmissionAdmission, Error>
where
    F: FnOnce(SubmissionRecord, AgentExecutionProfile) -> Fut,
    Fut: Future<Output = Result<Arc<dyn ModelTransport>, Error>>,
{
    start_prepared_retaining(
        store,
        sessions,
        request,
        WorkerRetention::default(),
        |record, profile| async move {
            Ok((
                factory(record, profile).await?,
                None,
                OwnedToolRegistration::new(
                    vec![],
                    Arc::new(super::composed_tools::RejectingToolExecutor),
                ),
            ))
        },
    )
    .await
}

type PreparedCompaction = (
    Arc<dyn ModelTransport>,
    super::compaction_hook_factory::PreparedCompactionHooks,
    OwnedToolRegistration,
);

async fn start_prepared_retaining<F, Fut>(
    store: &AppStore,
    sessions: Arc<AgentSessionService>,
    request: StartRequest,
    worker_retention: WorkerRetention,
    factory: F,
) -> Result<SubmissionAdmission, Error>
where
    F: FnOnce(SubmissionRecord, AgentExecutionProfile) -> Fut,
    Fut: Future<Output = Result<PreparedCompaction, Error>>,
{
    if request.conversation_id.is_nil()
        || request.request_id.is_nil()
        || request.context_snapshot_id.is_nil()
        || request.expected_revision > 9_007_199_254_740_991
    {
        return Err(Error::InvalidInput);
    }
    let record = restore(store, &request).await?;
    if record.context_snapshot_id != request.context_snapshot_id
        || record.input.model != request.model
        || record.input.text.trim() != "/compact"
    {
        return Err(Error::ConfirmationMismatch);
    }
    preparation::validate_frozen_fence(store, &request).await?;
    let submissions = Arc::new(SqliteSubmissionStore::for_app(store));
    if let Some(admitted) = sessions
        .existing_compaction(submissions.as_ref(), &record)
        .await?
    {
        return Ok(admitted);
    }
    // Reject an obsolete claim before credential/catalog preparation. The journal
    // repeats this under TurnStarted's transaction before model or hook execution.
    // The actual preflight worker, rather than its HTTP waiter, retains admission.
    queued_claim_worker(
        store.clone(),
        record.conversation_id,
        record.request_id,
        &worker_retention,
    )
    .await
    .map_err(|_| Error::HostUnavailable)??;
    let id = ConversationId::from_uuid(request.conversation_id);
    let actual = sessions
        .open(id)
        .await
        .map_err(|error| Error::Session(error.into()))?
        .seq();
    if actual != request.expected_revision {
        return Err(Error::HistoryChanged {
            expected: request.expected_revision,
            actual,
        });
    }
    let host = request.host.model_host();
    let profile = execution_profile_for_host(
        &record.input.model,
        execution_mode_for_model(
            &record.input.model,
            record.input.reasoning_effort.as_deref(),
        ),
        host,
    )
    .and_then(|profile| profile.with_submission_compaction(&record.input))
    .map_err(|_| Error::UnsupportedModel)?;
    let (inner, compaction_hooks, host_tools) = factory(record.clone(), profile.clone()).await?;
    // Binding can be removed while native credential preparation is in flight.
    if restore(store, &request).await? != record {
        return Err(Error::ConfirmationMismatch);
    }
    let transport: Arc<dyn ModelTransport> = Arc::new(BoundTransport {
        store: store.clone(),
        request: request.clone(),
        record,
        inner,
    });
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks,
        model: request.model,
        compaction_strategy: Some(
            profile
                .compaction_strategy(transport.clone())
                .map_err(|_| Error::UnsupportedModel)?,
        ),
        transport,
        host_tools,
        system_instructions: request.host.instructions().into(),
        limits: profile.limits,
        max_output_tokens: profile.max_output_tokens,
        max_input_bytes: profile.max_input_bytes,
    });
    execution
        .start_confirmed_compaction(
            sessions,
            submissions,
            CompactionStart {
                conversation_id: id,
                request_id: request.request_id,
                expected_revision: request.expected_revision,
            },
            Some(Arc::new(AppImageMaterializer::new(store))),
        )
        .await
        .map_err(Into::into)
}

fn queued_claim_worker(
    store: AppStore,
    conversation: Uuid,
    request: Uuid,
    retention: &WorkerRetention,
) -> tokio::task::JoinHandle<Result<(), Error>> {
    retention.spawn(move || {
        let db = store.connection().map_err(|_| Error::HostUnavailable)?;
        super::repository::input_queue::queued_compaction::require_startable_in(
            &db,
            conversation,
            request,
        )
        .map_err(|_| Error::ConfirmationMismatch)
    })
}

struct BoundTransport {
    store: AppStore,
    request: StartRequest,
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
    fn capabilities(&self) -> research_agent_core::model::ModelCapabilities {
        self.inner.capabilities()
    }
    fn supports_namespace_tools(&self) -> bool {
        self.inner.supports_namespace_tools()
    }
    fn authorize_request<'a>(&'a self, request: &'a ModelRequest) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            let saved = restore(&self.store, &self.request)
                .await
                .map_err(|_| ModelError::InvalidRequest)?;
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
#[path = "compaction_commands/test_fixtures.rs"]
pub(crate) mod test_fixtures;
#[cfg(test)]
#[path = "compaction_commands/tests.rs"]
mod tests;
