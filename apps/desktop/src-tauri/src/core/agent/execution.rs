//! Compose the shared runner with durable submissions and native ports. Provider
//! credentials, tool authorization and context materialization remain outside.
use super::{
    cancellation::{AgentCancellation, run_cancellable_turn_with_sampling_context},
    service::{AgentSessionService, SubmissionAdmission, SubmissionRunError},
    turn_sink::ServiceTurnSink,
};
use research_agent_core::{
    conversation::ConversationId,
    model::*,
    protocol::{ConversationProjection, EventPayload, ModelIdentity, TurnOutcome},
    runner::*,
    submission::{SubmissionRecord, SubmissionStore},
    tool_registry::{OwnedToolRegistration, ToolRegistration, ToolRegistry},
};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

pub struct AgentExecution {
    #[cfg(target_os = "macos")]
    pub(crate) stop_hooks: Option<NativeStopBinding>,
    pub compaction_hooks:
        Option<Arc<dyn research_agent_core::context::compaction_hooks::CompactionHooks>>,
    pub model: ModelIdentity,
    pub transport: Arc<dyn ModelTransport>,
    pub compaction_strategy:
        Option<Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy>>,
    pub host_tools: OwnedToolRegistration,
    pub system_instructions: String,
    pub limits: RunLimits,
    pub max_output_tokens: u32,
    pub max_input_bytes: usize,
}
#[cfg(target_os = "macos")]
pub(crate) struct NativeStopBinding {
    pub handlers: super::compaction_hook_config::native::NativeStopHandlers,
    pub audit: super::repository::stop_hook_audit::SqliteStopHookAudit,
    pub target: super::stop_hook_process::StopTarget,
}
#[cfg(target_os = "macos")]
impl NativeStopBinding {
    pub(crate) async fn prepare(
        models: &crate::ai::agent_configuration::AgentModelResolver,
        host: super::model::AgentHost,
        store: &crate::storage::AppStore,
        record: &SubmissionRecord,
        workspace: super::compaction_hook_factory::HookWorkspaceHandle,
    ) -> Result<Option<Self>, crate::ai::agent_configuration::AgentModelPreparationError> {
        // The durable child record selects the reference's ThreadSpawn event.
        // This application currently has no named child roles or transcript file.
        let db = store.shared_agent_connection();
        let conversation = record.conversation_id;
        let child = tokio::task::spawn_blocking(move || {
            let connection = db.lock().map_err(|_| ())?;
            super::repository::subtasks::find_by_child(&connection, conversation).map_err(|_| ())
        })
        .await
        .map_err(|_| crate::ai::agent_configuration::AgentModelPreparationError::Worker)?
        .map_err(|_| crate::ai::agent_configuration::AgentModelPreparationError::Settings)?;
        let target = match child {
            Some(child) => super::stop_hook_process::StopTarget::SubagentStop {
                agent_id: child.child_conversation_id.to_string(),
                agent_type: "default".into(),
                agent_transcript_path: None,
            },
            None => super::stop_hook_process::StopTarget::Stop,
        };
        Ok(models
            .prepare_stop_hooks(host, record, workspace, target.clone())
            .await?
            .map(|handlers| Self {
                handlers,
                audit: super::repository::stop_hook_audit::SqliteStopHookAudit::new(
                    store.shared_agent_connection(),
                ),
                target,
            }))
    }
}
struct NativeIds;
type RetainedUserImages = HashMap<Uuid, Vec<ContentPart>>;

struct RequestPreparation<'a> {
    compact_only: bool,
    signal: &'a AgentCancellation,
    materializer: Option<&'a dyn research_agent_core::submission::ImageMaterializer>,
    tool_definitions: &'a [ToolDefinition],
    canonical_out: Option<&'a mut Vec<ModelMessage>>,
    retained_images_out: &'a mut RetainedUserImages,
}

impl RunIdSource for NativeIds {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

struct TokioRetryWait {
    cancellation: AgentCancellation,
}
impl RetryWait for TokioRetryWait {
    fn wait(&self, delay_ms: u64) -> RunFuture<'_, ()> {
        Box::pin(async move {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => Ok(()),
                _ = self.cancellation.cancelled() => Err(ModelError::Canceled.into()),
            }
        })
    }
}
struct BudgetedTransport<'a> {
    inner: &'a dyn ModelTransport,
    limit: usize,
}
impl ModelTransport for BudgetedTransport<'_> {
    fn tool_output_history_prepared(&self) -> bool {
        true
    }
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
    fn authorize_request<'a>(&'a self, request: &'a ModelRequest) -> ModelFuture<'a, ()> {
        self.inner.authorize_request(request)
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
        self.inner.validate_compaction_window(checkpoint, request)
    }
    fn token_budget_policy(
        &self,
    ) -> Option<Arc<research_agent_core::context::token_budget::TokenBudgetPolicy>> {
        self.inner.token_budget_policy()
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
    fn start_text_compaction(
        &self,
        request: ModelRequest,
    ) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            check_budget(&request.messages, &request.tools, self.limit)
                .map_err(|_| ModelError::BudgetExceeded)?;
            if let Some(accounting) = self.inner.context_accounting() {
                accounting.check(&request.model, &request.messages, &request.tools)?;
            }
            self.inner.start_text_compaction(request).await
        })
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            check_budget(&request.messages, &request.tools, self.limit)
                .map_err(|_| ModelError::BudgetExceeded)?;
            if let Some(accounting) = self.inner.context_accounting() {
                accounting.check(&request.model, &request.messages, &request.tools)?;
            }
            self.inner.start(request).await
        })
    }
}

impl AgentExecution {
    fn consuming_checkpoint(
        &self,
        stored: Option<research_agent_core::context::checkpoint::ContextCheckpoint>,
    ) -> Result<Option<research_agent_core::context::checkpoint::ContextCheckpoint>, RunError> {
        stored
            .map(|mut window| {
                let strategy = self
                    .compaction_strategy
                    .as_ref()
                    .ok_or(ModelError::UnsupportedCapability)?;
                if !strategy.accepts_binding(&window.binding) {
                    return Err(ModelError::UnsupportedCapability.into());
                }
                window.binding.model = self.model.clone();
                Ok(window)
            })
            .transpose()
    }
    fn check_context_budget(
        &self,
        messages: &[ModelMessage],
        tools: &[ToolDefinition],
    ) -> Result<(), RunError> {
        self.check_context_window(messages, tools, None)
    }
    fn check_context_window(
        &self,
        messages: &[ModelMessage],
        tools: &[ToolDefinition],
        checkpoint: Option<&research_agent_core::context::checkpoint::ContextCheckpoint>,
    ) -> Result<(), RunError> {
        check_budget(messages, tools, self.context_input_limit(checkpoint)?)?;
        if let Some(accounting) = self.transport.context_accounting() {
            accounting.check_window(&self.model, messages, tools, checkpoint)?;
        }
        Ok(())
    }
    fn should_compact_context(
        &self,
        messages: &[ModelMessage],
        tools: &[ToolDefinition],
        checkpoint: Option<&research_agent_core::context::checkpoint::ContextCheckpoint>,
    ) -> Result<bool, RunError> {
        let token_trigger = if let Some(policy) = self.transport.token_budget_policy() {
            // The experimental buffer belongs to the frozen reset strategy,
            // never to a summary/opaque policy or a guessed provider capability.
            let binding = self
                .compaction_strategy
                .as_ref()
                .map(|strategy| strategy.binding());
            if !binding.is_some_and(|binding| {
                binding.kind == research_agent_core::context::compaction_strategy::CompactionKind::ResetWindow
                    && binding.model == self.model
            }) {
                return Err(ModelError::UnsupportedCapability.into());
            }
            let accounting = self
                .transport
                .context_accounting()
                .ok_or(ModelError::UnsupportedCapability)?;
            policy
                .status(&accounting, &self.model, messages, tools, checkpoint)?
                .token_limit_reached
        } else {
            self.transport
                .context_accounting()
                .map(|accounting| {
                    accounting.should_compact_window(&self.model, messages, tools, checkpoint)
                })
                .transpose()?
                .unwrap_or(false)
        };
        Ok(token_trigger)
    }
}

impl AgentExecution {
    pub async fn start(
        self: Arc<Self>,
        service: Arc<AgentSessionService>,
        submissions: Arc<dyn SubmissionStore>,
        id: ConversationId,
        request_id: Uuid,
    ) -> Result<SubmissionAdmission, SubmissionRunError> {
        self.start_with_steering(service, submissions, id, request_id, None)
            .await
    }

    /// Start a turn with explicitly materialized image parts.
    pub async fn start_with_images(
        self: Arc<Self>,
        service: Arc<AgentSessionService>,
        submissions: Arc<dyn SubmissionStore>,
        materializer: Arc<dyn research_agent_core::submission::ImageMaterializer>,
        id: ConversationId,
        request_id: Uuid,
    ) -> Result<SubmissionAdmission, SubmissionRunError> {
        self.start_with_images_and_steering(
            service,
            submissions,
            materializer,
            id,
            request_id,
            None,
        )
        .await
    }

    pub async fn start_with_images_and_steering(
        self: Arc<Self>,
        service: Arc<AgentSessionService>,
        submissions: Arc<dyn SubmissionStore>,
        materializer: Arc<dyn research_agent_core::submission::ImageMaterializer>,
        id: ConversationId,
        request_id: Uuid,
        steering: Option<Arc<dyn research_agent_core::steering::SteeringStore>>,
    ) -> Result<SubmissionAdmission, SubmissionRunError> {
        let worker = service.clone();
        let store = submissions.clone();
        service
            .start_submission(
                submissions.as_ref(),
                id,
                request_id,
                move |record, history, signal| async move {
                    self.run(
                        &worker,
                        store.as_ref(),
                        record,
                        history,
                        signal,
                        steering.as_deref(),
                        Some(materializer.as_ref()),
                    )
                    .await
                },
            )
            .await
    }

    pub async fn start_with_steering(
        self: Arc<Self>,
        service: Arc<AgentSessionService>,
        submissions: Arc<dyn SubmissionStore>,
        id: ConversationId,
        request_id: Uuid,
        steering: Option<Arc<dyn research_agent_core::steering::SteeringStore>>,
    ) -> Result<SubmissionAdmission, SubmissionRunError> {
        let worker = service.clone();
        let store = submissions.clone();
        service
            .start_submission(
                submissions.as_ref(),
                id,
                request_id,
                move |record, history, signal| async move {
                    self.run(
                        &worker,
                        store.as_ref(),
                        record,
                        history,
                        signal,
                        steering.as_deref(),
                        None,
                    )
                    .await
                },
            )
            .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "runner boundary keeps durable inputs and optional image port explicit"
    )]
    async fn run(
        &self,
        service: &AgentSessionService,
        submissions: &dyn SubmissionStore,
        record: SubmissionRecord,
        history: ConversationProjection,
        signal: AgentCancellation,
        steering: Option<&dyn research_agent_core::steering::SteeringStore>,
        materializer: Option<&dyn research_agent_core::submission::ImageMaterializer>,
    ) -> Result<(), RunError> {
        let sink = ServiceTurnSink {
            service,
            conversation_id: ConversationId::from_uuid(record.conversation_id),
            request_id: record.request_id,
        };
        let token_budget =
            token_budget_tools::NativeTokenBudgetTools::new(self, service, &record, &signal);
        let budget_tools = research_agent_core::context::token_budget::tools::TokenBudgetTools::new(token_budget.as_ref().map(|runtime| runtime as &dyn research_agent_core::context::token_budget::tools::TokenBudgetToolRuntime));
        let source_tools = super::source_tools::SubmissionTools { record: &record };
        let tool_registry = match ToolRegistry::from_registrations([
            self.host_tools.borrowed(),
            budget_tools.registration(),
            ToolRegistration::new(
                if record.input.context.iter().any(|source| {
                    super::context_session::resource_grants::resource(source).is_some()
                }) {
                    vec![]
                } else {
                    super::source_tools::definitions(&record)
                },
                &source_tools,
            ),
        ]) {
            Ok(registry) => registry,
            Err(_) => {
                sink.emit(EventPayload::TurnFinished {
                    turn_id: record.turn_id,
                    outcome: TurnOutcome::Failed,
                })
                .await?;
                return Err(ModelError::InvalidRequest.into());
            }
        };
        // Preparation belongs to the turn's terminal-event boundary too. A
        // missing image is a known failure, not an interrupted durable commit.
        let mut canonical = Vec::new();
        let mut retained_images = HashMap::new();
        let request = async {
            if signal.is_cancelled() {
                return Err(RunError::Model(ModelError::Canceled));
            }
            if token_budget.is_some() {
                service
                    .prepare_token_budget_window(record.conversation_id, record.turn_id)
                    .await
                    .map_err(|error| match error {
                        research_agent_core::context::checkpoint_store::CheckpointStoreError::PersistenceDiagnostic(detail) => RunError::from(ModelError::PersistenceDiagnostic(detail)),
                        _ => RunError::EventSink,
                    })?;
            }
            self.request(
                service,
                submissions,
                &record,
                &history,
                RequestPreparation {
                    compact_only: false,
                    signal: &signal,
                    materializer,
                    tool_definitions: tool_registry.definitions(),
                    canonical_out: Some(&mut canonical),
                    retained_images_out: &mut retained_images,
                },
            )
            .await
        }
        .await;
        let request = match request {
            Ok(request) => request,
            Err(error) if error.is_event_sink() => return Err(error),
            Err(error) => {
                let outcome = if matches!(error, RunError::Model(ModelError::Canceled)) {
                    TurnOutcome::Canceled
                } else {
                    TurnOutcome::Failed
                };
                let attempt_id = Uuid::new_v4();
                sink.emit(EventPayload::PhaseChanged {
                    turn_id: record.turn_id,
                    phase: research_agent_core::protocol::TurnPhase::RunningModel,
                })
                .await?;
                sink.emit(EventPayload::ModelRequestStarted {
                    turn_id: record.turn_id,
                    attempt_id,
                    model: record.input.model.clone(),
                })
                .await?;
                // Keep preparation failures diagnosable without persisting provider
                // payloads or credentials. ModelRequestFailed is already part of
                // the shared event protocol and is the same failure boundary used
                // for transport errors later in this runner.
                sink.emit(EventPayload::ModelRequestFailed {
                    turn_id: record.turn_id,
                    attempt_id,
                    error: error.to_string(),
                    retryable: false,
                    category: Some(error.failure_category()),
                })
                .await?;
                sink.emit(EventPayload::TurnFinished {
                    turn_id: record.turn_id,
                    outcome,
                })
                .await?;
                return Err(error);
            }
        };
        let transport = BudgetedTransport {
            inner: self.transport.as_ref(),
            limit: self.max_input_bytes,
        };
        let input = steering.map(|store| super::service::SessionTurnInput {
            evidence_store: Some(submissions),
            injected: std::sync::Mutex::new(
                record
                    .input
                    .skill_instructions
                    .iter()
                    .map(|prompt| prompt.identity.clone())
                    .collect(),
            ),
            service,
            store,
            scope: research_agent_core::steering::SteeringScope {
                conversation_id: record.conversation_id,
                turn_id: record.turn_id,
            },
        });
        #[cfg(target_os = "macos")]
        let stop_runtime = self.stop_hooks.as_ref().map(|binding| {
            super::compaction_hook_config::native::NativeStopRuntime {
                handlers: &binding.handlers,
                service,
                audit: &binding.audit,
                record: &record,
                signal: &signal,
                permission_mode: "default",
                target: binding.target.clone(),
            }
        });
        let sampling = sampling::NativeSamplingContext {
            execution: self,
            service,
            submissions,
            record: &record,
            signal: &signal,
            canonical: &canonical,
            retained_images: &retained_images,
            token_budget: token_budget.as_ref(),
            #[cfg(target_os = "macos")]
            stop_hooks: stop_runtime
                .as_ref()
                .map(|runtime| runtime as &dyn StopHooks),
        };
        run_cancellable_turn_with_sampling_context(
            TurnRunner {
                transport: &transport,
                tools: &tool_registry,
                sink: &sink,
                ids: &NativeIds,
                limits: self.limits,
            },
            record.turn_id,
            request,
            signal.clone(),
            input.as_ref().map(|input| input as &dyn TurnInputSource),
            Some(&TokioRetryWait {
                cancellation: signal.clone(),
            }),
            Some(&sampling),
        )
        .await?;
        Ok(())
    }
}

fn image_error(error: research_agent_core::submission::SubmissionError) -> ModelError {
    match error {
        research_agent_core::submission::SubmissionError::Unavailable => ModelError::Unavailable,
        _ => ModelError::InvalidRequest,
    }
}
// Content-byte guard, not a tokenizer or an estimate of serialized HTTP size.
fn check_budget(
    messages: &[ModelMessage],
    tools: &[ToolDefinition],
    limit: usize,
) -> Result<(), RunError> {
    let mut remaining = limit;
    let mut consume = |bytes: usize| {
        remaining = remaining.checked_sub(bytes).ok_or(RunError::Budget)?;
        Ok::<_, RunError>(())
    };
    for tool in tools {
        consume(tool.name.len())?;
        consume(tool.namespace.as_ref().map_or(0, String::len))?;
        consume(tool.description.len())?;
        consume(tool.parameters.to_string().len())?;
    }
    consume(message_bytes(messages).ok_or(RunError::Budget)?)?;
    Ok(())
}
#[path = "execution/context_parts.rs"]
mod context_parts;
use context_parts::{context_parts, incremental_context_parts};
#[path = "execution/resource_context.rs"]
pub(crate) mod resource_context;

#[path = "execution/compaction.rs"]
mod compaction;
#[path = "execution/compaction_hooks.rs"]
mod compaction_hooks;
#[path = "execution/context_budget.rs"]
mod context_budget;
#[path = "execution/manual_compaction.rs"]
mod manual_compaction;
pub use manual_compaction::CompactionStart;
#[path = "execution/request.rs"]
mod request;
#[path = "execution/request_bytes.rs"]
mod request_bytes;
#[path = "execution/request_history_ref.rs"]
mod request_history_ref;
use request_bytes::message_bytes;
#[path = "execution/sampling.rs"]
mod sampling;
#[path = "execution/sampling_skills.rs"]
mod sampling_skills;
#[path = "execution/user_images.rs"]
mod user_images;
use user_images::{evidence_parts, image_parts, original_user_item};
#[path = "execution/token_budget_tools.rs"]
mod token_budget_tools;

#[cfg(test)]
#[path = "execution/tests.rs"]
mod tests;
