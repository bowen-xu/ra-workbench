//! Production app-config composition. Freeze commands/consent before admission;
//! acquire the original workspace only within the task-owned hook invocation.
//! Reuses the Codex-derived concurrent dispatcher and native process supervisor.
use super::*;
use crate::agent::{
    compaction_hook_commands::{HookConsentService, Snapshot},
    compaction_hook_factory::PreparedCompactionHooks,
    compaction_hook_spill::HookContextBudget,
    instructions::InstructionRootIdentity,
    model::AgentHost,
    repository::compaction_hook_trust::GrantKey,
};
use consent::LiveHookGrant;
use research_agent_core::{
    model::{ModelError, ModelFuture},
    submission::SubmissionRecord,
};
#[path = "native/stop.rs"]
mod stop;
#[path = "native/workspace.rs"]
mod workspace;
pub(crate) use stop::runtime::NativeStopRuntime;
pub use stop::{
    NativeStopHandlers, RecordedStopRun, StopCollectedOutcome, StopHandlerCollector,
    StopHandlerNotification, StopHandlerObserver, StopHandlerResult, StopPromptPart,
    StopRecordedError, StopResolution,
};
pub use workspace::{HookWorkspace, HookWorkspaceSource};

pub struct NativeHookFactory {
    settings: Arc<HookConsentService>,
}
impl NativeHookFactory {
    pub(crate) fn new(settings: Arc<HookConsentService>) -> Self {
        Self { settings }
    }
    pub fn prepare<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
        workspace: Option<Arc<dyn HookWorkspaceSource>>,
        budget: Option<HookContextBudget>,
    ) -> ModelFuture<'a, PreparedCompactionHooks> {
        let settings = self.settings.clone();
        let record = Arc::new(record.clone());
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let Some(snapshot) = settings.snapshot().map_err(|_| ModelError::Unavailable)?
                else {
                    return Ok(None);
                };
                if !snapshot
                    .configuration
                    .handlers()
                    .iter()
                    .any(|handler| handler.enabled && handler.stage.compaction().is_some())
                {
                    return Ok(None);
                }
                let workspace = workspace.ok_or(ModelError::Unavailable)?;
                let budget = budget.ok_or(ModelError::Unavailable)?;
                let Snapshot {
                    path,
                    device,
                    inode,
                    configuration,
                } = snapshot;
                let revision = configuration.revision.clone();
                let mut handlers = Vec::new();
                // The app's original workspace has an exclusive persistent
                // writer intent. Keep concurrent dispatch/audit, but await this
                // task-owned gate before native commands touch the same copy.
                let writer = Arc::new(tokio::sync::Mutex::new(()));
                for handler in configuration
                    .handlers
                    .into_iter()
                    .filter(|handler| handler.enabled && handler.stage.compaction().is_some())
                {
                    let stage = handler.stage.compaction().ok_or(ModelError::Unavailable)?;
                    let key = GrantKey::new(&path, handler.id, host, record.input.model.clone())
                        .map_err(|_| ModelError::Unavailable)?;
                    let grant = settings
                        .consent()
                        .load(&key)
                        .map_err(|_| ModelError::Unavailable)?;
                    let state = Arc::new(Authority {
                        id: handler.id,
                        stage,
                        revision: revision.clone(),
                        path: path.clone(),
                        device,
                        inode,
                        key,
                        grant,
                        record: record.clone(),
                    });
                    if handler.matcher.is_none() && !state.allowed() {
                        return Err(ModelError::Unavailable);
                    }
                    handlers.push(CompactionHookHandler {
                        id: handler.id,
                        stage,
                        hooks: Arc::new(DeferredHook {
                            handler,
                            state,
                            workspace: workspace.clone(),
                            writer: writer.clone(),
                        }),
                    });
                }
                let hooks = ConcurrentCompactionHooks::new(handlers, Arc::new(QuietObserver))
                    .map_err(|_| ModelError::Unavailable)?;
                Ok(Some(
                    budget
                        .wrap(
                            &record.input.model,
                            Arc::new(hooks),
                            settings.output_store(),
                        )
                        .map_err(|_| ModelError::Unavailable)?,
                ))
            })
            .await
            .map_err(|_| ModelError::Unavailable)?
        })
    }
}

struct QuietObserver;
impl HookHandlerObserver for QuietObserver {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, _: CompactionHookNotification) {}
}
struct Authority {
    id: Uuid,
    stage: CompactionHookStage,
    revision: String,
    path: PathBuf,
    device: u64,
    inode: u64,
    key: GrantKey,
    grant: Arc<LiveHookGrant>,
    record: Arc<SubmissionRecord>,
}
impl Authority {
    fn allowed(&self) -> bool {
        self.grant.allows(
            &self.key,
            &InstructionRootIdentity {
                canonical_path: &self.path,
                device: self.device,
                inode: self.inode,
            },
            &self.revision,
        )
    }
}
impl HookInvocationAuthority for Authority {
    fn check(
        &self,
        id: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        (id == self.id
            && revision == self.revision
            && request.stage == self.stage
            && request.model == self.record.input.model
            && request.conversation_id == self.record.conversation_id
            && request.turn_id == self.record.turn_id
            && !request.operation_id.is_nil()
            && self.allowed())
        .then_some(())
        .ok_or(HookConfigError)
    }
}
struct DeferredHook {
    handler: ConfiguredHook,
    state: Arc<Authority>,
    workspace: Arc<dyn HookWorkspaceSource>,
    writer: Arc<tokio::sync::Mutex<()>>,
}
impl DeferredHook {
    async fn execute(&self, request: CompactionHookRequest) -> CompactionHookResult {
        let failed = || CompactionHookResult::without_context(CompactionHookOutcome::Failed);
        let _writer = self.writer.lock().await;
        if self
            .state
            .check(self.handler.id, &self.state.revision, &request)
            .is_err()
        {
            return failed();
        }
        let Ok(workspace) = self.workspace.prepare(&self.state.record).await else {
            return failed();
        };
        let Ok(process) = self.handler.candidate_in(&workspace.directory) else {
            return failed();
        };
        let state = self.state.clone();
        let launcher = workspace.launcher(Arc::new(move |_, request| {
            state
                .check(state.id, &state.revision, request)
                .map_err(|_| HookCommandError)
        }));
        let source = Arc::new(FrozenCommand {
            handler_id: self.handler.id,
            revision: self.state.revision.clone(),
            authority: self.state.clone(),
            model: self.state.record.input.model.clone(),
            stage: request.stage,
            plan: AuthorizedHookCommand {
                process,
                transcript_path: None,
                launcher: Some(launcher),
            },
        });
        let Ok(hook) =
            ProcessCompactionHooks::new(source, self.handler.timeout, self.handler.output_bytes)
        else {
            return failed();
        };
        hook.run_with_context(request).await
    }
}
impl CompactionHooks for DeferredHook {
    fn accepts_request(&self, request: &CompactionHookRequest) -> bool {
        let value = match request.stage {
            CompactionHookStage::SessionStartCompact => "compact",
            _ if request.phase
                == research_agent_core::context::compaction_strategy::CompactionPhase::Manual =>
            {
                "manual"
            }
            _ => "auto",
        };
        matches_hook_matcher(self.handler.matcher.as_deref(), value)
    }
    fn validate_request(&self, request: &CompactionHookRequest) -> Result<(), HookContextError> {
        if self.handler.matcher.is_none() {
            return Ok(());
        }
        self.state
            .check(self.handler.id, &self.state.revision, request)
            .map_err(|_| HookContextError)
    }
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async move { self.execute(request).await.outcome })
    }
    fn run_with_context(&self, request: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        Box::pin(self.execute(request))
    }
}

#[cfg(test)]
#[path = "native/tests.rs"]
mod tests;
