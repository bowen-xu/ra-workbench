//! Stop handler preparation reuses the app's existing config, consent and
//! workspace owner. Execution is task-owned; durable admission remains caller-owned.
use super::*;
use crate::agent::stop_hook_process::{
    AuthorizedStopCommand, ProcessStopHook, StopCommandSource, StopRequest, StopTarget,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use research_agent_core::runner::{StopEvent, StopHandlerOutput, StopHandlerStatus};
use std::collections::HashSet;
use std::future::Future;
#[path = "stop/collector.rs"]
mod collector;
#[path = "stop/commit.rs"]
pub(super) mod commit;
#[path = "stop/prompt.rs"]
mod prompt;
#[path = "stop/runtime.rs"]
pub(super) mod runtime;
pub use collector::{StopCollectedOutcome, StopHandlerCollector};
pub use prompt::{StopPromptPart, StopResolution};

pub struct StopHandlerResult {
    pub handler_id: Uuid,
    pub run_id: Uuid,
    pub output: StopHandlerOutput,
}

/// Synchronous, task-owned collection only. Implementations must not perform
/// storage or launch detached work; the owner awaits persistence afterward.
pub enum StopHandlerNotification<'a> {
    Started,
    Completed(&'a StopHandlerOutput),
    Canceled,
}
pub trait StopHandlerObserver: Send + Sync {
    fn notify(&self, handler_id: Uuid, run_id: Uuid, notification: StopHandlerNotification<'_>);
}
#[cfg(test)]
struct QuietObserver;
#[cfg(test)]
impl StopHandlerObserver for QuietObserver {
    fn notify(&self, _: Uuid, _: Uuid, _: StopHandlerNotification<'_>) {}
}

pub struct NativeStopHandlers {
    handlers: Vec<DeferredStop>,
    event: StopEvent,
    target: Option<StopTarget>,
}

#[derive(Debug, thiserror::Error)]
pub enum StopRecordedError {
    #[error("stop hook persistence failed (invocation {invocation_id})")]
    Storage {
        invocation_id: Uuid,
        #[source]
        source: rusqlite::Error,
    },
    #[error("stop hook dispatch failed (invocation {invocation_id:?})")]
    Dispatch { invocation_id: Option<Uuid> },
    #[error("stop hook clock unavailable (invocation {0})")]
    Clock(Uuid),
}
pub struct RecordedStopRun {
    pub invocation_id: Uuid,
    pub canceled: bool,
    pub outcomes: Vec<(
        Uuid,
        Uuid,
        crate::agent::repository::stop_hook_audit::Outcome,
    )>,
}
impl RecordedStopRun {
    /// Only describes the committed result. The task must durably insert a
    /// source-bearing conversation item before sending a FollowUp to a model.
    pub fn resolution(&self) -> StopResolution {
        prompt::resolution(self.canceled, &self.outcomes)
    }
}

fn timestamp(invocation_id: Uuid) -> Result<i64, StopRecordedError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .ok_or(StopRecordedError::Clock(invocation_id))
}

impl NativeHookFactory {
    /// Freeze the same source/consent identities as compaction, without opening
    /// a workspace or executing commands during preparation.
    #[cfg(test)]
    pub fn prepare_stop<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
        workspace: Option<Arc<dyn HookWorkspaceSource>>,
        event: StopEvent,
    ) -> ModelFuture<'a, Option<NativeStopHandlers>> {
        self.prepare_stop_inner(host, record, workspace, event, None)
    }

    pub fn prepare_stop_target<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
        workspace: Option<Arc<dyn HookWorkspaceSource>>,
        target: StopTarget,
    ) -> ModelFuture<'a, Option<NativeStopHandlers>> {
        self.prepare_stop_inner(host, record, workspace, target_event(&target), Some(target))
    }

    fn prepare_stop_inner<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
        workspace: Option<Arc<dyn HookWorkspaceSource>>,
        event: StopEvent,
        target: Option<StopTarget>,
    ) -> ModelFuture<'a, Option<NativeStopHandlers>> {
        let settings = self.settings.clone();
        let record = Arc::new(record.clone());
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let Some(snapshot) = settings.snapshot().map_err(|_| ModelError::Unavailable)?
                else {
                    return Ok(None);
                };
                if !snapshot.configuration.handlers().iter().any(|h| {
                    h.enabled
                        && matches_event(h.stage, event)
                        && target
                            .as_ref()
                            .is_none_or(|target| matches_handler(h, target))
                }) {
                    return Ok(None);
                }
                let workspace = workspace.ok_or(ModelError::Unavailable)?;
                let Snapshot {
                    path,
                    device,
                    inode,
                    configuration,
                } = snapshot;
                let revision = configuration.revision.clone();
                let writer = Arc::new(tokio::sync::Mutex::new(()));
                let mut handlers = Vec::new();
                for handler in configuration.handlers.into_iter().filter(|h| {
                    h.enabled
                        && matches_event(h.stage, event)
                        && target
                            .as_ref()
                            .is_none_or(|target| matches_handler(h, target))
                }) {
                    let key = GrantKey::new(&path, handler.id, host, record.input.model.clone())
                        .map_err(|_| ModelError::Unavailable)?;
                    let grant = settings
                        .consent()
                        .load(&key)
                        .map_err(|_| ModelError::Unavailable)?;
                    let authority = Arc::new(StopAuthority {
                        id: handler.id,
                        stage: handler.stage,
                        revision: revision.clone(),
                        path: path.clone(),
                        device,
                        inode,
                        key,
                        grant,
                        record: record.clone(),
                    });
                    if !authority.allowed() {
                        return Err(ModelError::Unavailable);
                    }
                    handlers.push(DeferredStop {
                        handler,
                        authority,
                        workspace: workspace.clone(),
                        writer: writer.clone(),
                    });
                }
                Ok(Some(NativeStopHandlers {
                    handlers,
                    event,
                    target,
                }))
            })
            .await
            .map_err(|_| ModelError::Unavailable)?
        })
    }
}

impl NativeStopHandlers {
    pub fn handler_ids(&self, target: &StopTarget) -> Vec<Uuid> {
        if self.target.as_ref().is_some_and(|frozen| frozen != target) {
            return Vec::new();
        }
        self.handlers
            .iter()
            .filter(|h| matches_handler(&h.handler, target))
            .map(|h| h.handler.id)
            .collect()
    }
    /// The task awaits admission before dispatch and finish before consuming
    /// feedback. Cancellation never drops either storage transaction.
    pub async fn run_recorded(
        &self,
        request: &StopRequest,
        audit: &crate::agent::repository::stop_hook_audit::SqliteStopHookAudit,
        cancellation: impl Future<Output = ()> + Send,
    ) -> Result<Option<RecordedStopRun>, StopRecordedError> {
        use crate::agent::repository::stop_hook_audit::{Admission, Finished};
        if self.event != target_event(&request.target)
            || self
                .target
                .as_ref()
                .is_some_and(|target| target != &request.target)
        {
            return Err(StopRecordedError::Dispatch {
                invocation_id: None,
            });
        }
        let runs: Vec<_> = self
            .handler_ids(&request.target)
            .into_iter()
            .map(|handler| (handler, Uuid::new_v4()))
            .collect();
        if runs.is_empty() {
            return Ok(None);
        }
        if self
            .handlers
            .iter()
            .filter(|handler| matches_handler(&handler.handler, &request.target))
            .any(|handler| {
                handler.handler.id != handler.authority.id
                    || handler.authority.check(request).is_err()
            })
        {
            return Err(StopRecordedError::Dispatch {
                invocation_id: None,
            });
        }
        let admission = Admission {
            invocation_id: Uuid::new_v4(),
            conversation_id: request.conversation_id,
            turn_id: request.turn_id,
            event: match request.target {
                StopTarget::Stop => StopEvent::Stop,
                StopTarget::SubagentStop { .. } => StopEvent::SubagentStop,
            },
            model: request.model.clone(),
            handlers: runs.clone(),
        };
        let collector =
            StopHandlerCollector::new(runs.clone()).ok_or(StopRecordedError::Dispatch {
                invocation_id: Some(admission.invocation_id),
            })?;
        audit
            .admit(admission.clone(), timestamp(admission.invocation_id)?)
            .await
            .map_err(|source| StopRecordedError::Storage {
                invocation_id: admission.invocation_id,
                source,
            })?;
        let canceled = {
            let run = self.run_with_observer(request, &runs, &collector);
            tokio::pin!(run, cancellation);
            tokio::select! {
                biased;
                _ = &mut cancellation => true,
                result = &mut run => { result.map_err(|_| StopRecordedError::Dispatch { invocation_id: Some(admission.invocation_id) })?; false }
            }
        };
        let outcomes = collector
            .outcomes(canceled)
            .ok_or(StopRecordedError::Dispatch {
                invocation_id: Some(admission.invocation_id),
            })?
            .into_iter()
            .map(|(handler, run, outcome)| (handler, run, outcome.into()))
            .collect::<Vec<_>>();
        audit
            .finish(
                admission.clone(),
                Finished {
                    canceled,
                    outcomes: outcomes.clone(),
                },
                timestamp(admission.invocation_id)?,
            )
            .await
            .map_err(|source| StopRecordedError::Storage {
                invocation_id: admission.invocation_id,
                source,
            })?;
        Ok(Some(RecordedStopRun {
            invocation_id: admission.invocation_id,
            canceled,
            outcomes,
        }))
    }
    /// The owner must durably admit these exact unique run identities before
    /// calling. Return in configuration order even if executions finish out of order.
    #[cfg(test)]
    pub async fn run(
        &self,
        request: &StopRequest,
        runs: &[(Uuid, Uuid)],
    ) -> Result<Vec<StopHandlerResult>, ModelError> {
        self.run_with_observer(request, runs, &QuietObserver).await
    }
    /// Completed outcomes are observed before another handler can finish or
    /// cancellation drops this dispatch. Unpolled admissions remain the
    /// caller's responsibility and are never reported as started.
    pub async fn run_with_observer(
        &self,
        request: &StopRequest,
        runs: &[(Uuid, Uuid)],
        observer: &dyn StopHandlerObserver,
    ) -> Result<Vec<StopHandlerResult>, ModelError> {
        if self.event != target_event(&request.target)
            || self
                .target
                .as_ref()
                .is_some_and(|target| target != &request.target)
        {
            return Err(ModelError::InvalidRequest);
        }
        let selected: Vec<_> = self
            .handlers
            .iter()
            .filter(|h| matches_handler(&h.handler, &request.target))
            .collect();
        let expected: Vec<_> = selected.iter().map(|h| h.handler.id).collect();
        let mut ids = HashSet::new();
        if runs.len() != selected.len()
            || runs.iter().zip(expected).any(|((handler, run), expected)| {
                *handler != expected || run.is_nil() || !ids.insert(*run)
            })
        {
            return Err(ModelError::InvalidRequest);
        }
        let mut pending = FuturesUnordered::new();
        for (index, (handler, (handler_id, run_id))) in
            selected.into_iter().zip(runs.iter().copied()).enumerate()
        {
            pending.push(async move {
                let mut guard = Observation::start(observer, handler_id, run_id);
                let output = handler.execute(request).await;
                guard.complete(&output);
                (
                    index,
                    StopHandlerResult {
                        handler_id,
                        run_id,
                        output,
                    },
                )
            });
        }
        let mut results = Vec::new();
        while let Some(result) = pending.next().await {
            results.push(result);
        }
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, result)| result).collect())
    }
}

struct Observation<'a> {
    observer: &'a dyn StopHandlerObserver,
    handler_id: Uuid,
    run_id: Uuid,
    complete: bool,
}
impl<'a> Observation<'a> {
    fn start(observer: &'a dyn StopHandlerObserver, handler_id: Uuid, run_id: Uuid) -> Self {
        observer.notify(handler_id, run_id, StopHandlerNotification::Started);
        Self {
            observer,
            handler_id,
            run_id,
            complete: false,
        }
    }
    fn complete(&mut self, output: &StopHandlerOutput) {
        self.observer.notify(
            self.handler_id,
            self.run_id,
            StopHandlerNotification::Completed(output),
        );
        self.complete = true;
    }
}
impl Drop for Observation<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.observer.notify(
                self.handler_id,
                self.run_id,
                StopHandlerNotification::Canceled,
            );
        }
    }
}

fn matches_target(stage: ConfiguredHookStage, target: &StopTarget) -> bool {
    matches!(
        (stage, target),
        (ConfiguredHookStage::Stop, StopTarget::Stop)
            | (
                ConfiguredHookStage::SubagentStop,
                StopTarget::SubagentStop { .. }
            )
    )
}

fn matches_handler(handler: &ConfiguredHook, target: &StopTarget) -> bool {
    matches_target(handler.stage, target)
        && match target {
            StopTarget::Stop => true,
            StopTarget::SubagentStop { agent_type, .. } => {
                matches_hook_matcher(handler.matcher.as_deref(), agent_type)
            }
        }
}

fn matches_event(stage: ConfiguredHookStage, event: StopEvent) -> bool {
    matches!(
        (stage, event),
        (ConfiguredHookStage::Stop, StopEvent::Stop)
            | (ConfiguredHookStage::SubagentStop, StopEvent::SubagentStop)
    )
}

fn target_event(target: &StopTarget) -> StopEvent {
    match target {
        StopTarget::Stop => StopEvent::Stop,
        StopTarget::SubagentStop { .. } => StopEvent::SubagentStop,
    }
}

struct StopAuthority {
    id: Uuid,
    stage: ConfiguredHookStage,
    revision: String,
    path: PathBuf,
    device: u64,
    inode: u64,
    key: GrantKey,
    grant: Arc<LiveHookGrant>,
    record: Arc<SubmissionRecord>,
}
impl StopAuthority {
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
    fn check(&self, request: &StopRequest) -> Result<(), HookCommandError> {
        (matches_target(self.stage, &request.target)
            && request.model == self.record.input.model
            && request.conversation_id == self.record.conversation_id
            && request.turn_id == self.record.turn_id
            && self.allowed())
        .then_some(())
        .ok_or(HookCommandError)
    }
}
struct DeferredStop {
    handler: ConfiguredHook,
    authority: Arc<StopAuthority>,
    workspace: Arc<dyn HookWorkspaceSource>,
    writer: Arc<tokio::sync::Mutex<()>>,
}
impl DeferredStop {
    async fn execute(&self, request: &StopRequest) -> StopHandlerOutput {
        let fail = || StopHandlerOutput {
            status: StopHandlerStatus::Failed {
                diagnostic: "stop hook authorization or workspace unavailable".into(),
            },
            system_message: None,
        };
        let _writer = self.writer.lock().await;
        if self.handler.id != self.authority.id || self.authority.check(request).is_err() {
            return fail();
        }
        let Ok(workspace) = self.workspace.prepare(&self.authority.record).await else {
            return fail();
        };
        let Ok(process) = self.handler.candidate_in(&workspace.directory) else {
            return fail();
        };
        let authority = self.authority.clone();
        let launcher =
            workspace.stop_launcher(Arc::new(move |_, request| authority.check(request)));
        let source = Arc::new(FrozenStop {
            process,
            launcher,
            authority: self.authority.clone(),
        });
        let Ok(hook) =
            ProcessStopHook::new(source, self.handler.timeout, self.handler.output_bytes)
        else {
            return fail();
        };
        hook.run(request).await
    }
}
struct FrozenStop {
    process: ProcessSpec,
    launcher: Arc<dyn crate::agent::stop_hook_process::StopHookLauncher>,
    authority: Arc<StopAuthority>,
}
impl StopCommandSource for FrozenStop {
    fn prepare(
        &self,
        request: &StopRequest,
    ) -> Result<Option<AuthorizedStopCommand>, HookCommandError> {
        self.authority.check(request)?;
        Ok(Some(AuthorizedStopCommand {
            process: ProcessSpec {
                program: self.process.program.clone(),
                arguments: self.process.arguments.clone(),
                directory: self.process.directory.clone(),
                environment: self.process.environment.clone(),
            },
            transcript_path: None,
            launcher: self.launcher.clone(),
        }))
    }
}
