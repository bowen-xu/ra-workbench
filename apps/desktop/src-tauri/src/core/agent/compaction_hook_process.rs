//! Native command adapter for the shared compaction hook port. Protocol adapted
//! from Codex hooks/src/events/compact.rs and engine/command_runner.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b (Apache-2.0, Copyright 2025 OpenAI).
//! Uses the existing process supervisor; admission and sandboxing remain host-owned.
use super::process::{
    PipeInput, ProcessError, ProcessLimits, ProcessOutcome, ProcessSpec, StopReason,
    SupervisedProcess,
};
use research_agent_core::context::{compaction_hooks::*, compaction_strategy::CompactionPhase};
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Native-only authorized plan. Never deserialize a command from model or IPC
/// input. The host must validate workspace, executable, environment and sandbox.
pub struct AuthorizedHookCommand {
    pub process: ProcessSpec,
    pub transcript_path: Option<PathBuf>,
    pub launcher: Option<Arc<dyn HookProcessLauncher>>,
}

/// Native workspace admission. Implementations must recheck scope, sandbox and
/// acquire the original lease's writer intent; they never retry unsandboxed.
pub type HookLaunchFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<(SupervisedProcess, super::process::PipeInput), HookCommandError>,
            > + Send
            + 'a,
    >,
>;
pub trait HookProcessLauncher: Send + Sync {
    fn launch<'a>(
        &'a self,
        request: &'a CompactionHookRequest,
        process: ProcessSpec,
        limits: ProcessLimits,
    ) -> HookLaunchFuture<'a>;
}

/// Resolves one frozen authorized command for a stage. None means unconfigured.
/// Must not launch I/O, block or perform side effects while resolving the plan.
pub trait CompactionHookCommandSource: Send + Sync {
    fn prepare(
        &self,
        request: &CompactionHookRequest,
    ) -> Result<Option<AuthorizedHookCommand>, HookCommandError>;
}

#[derive(Debug, thiserror::Error)]
#[error("invalid or unavailable compaction hook command")]
pub struct HookCommandError;

/// Inject into AgentExecution.compaction_hooks; no default commands are enabled.
/// Notifications and durable audit are still owned by the existing task runner.
pub struct ProcessCompactionHooks {
    source: Arc<dyn CompactionHookCommandSource>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl ProcessCompactionHooks {
    pub fn new(
        source: Arc<dyn CompactionHookCommandSource>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<Self, HookCommandError> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(24 * 60 * 60)
            || max_output_bytes == 0
            || max_output_bytes > 1024 * 1024
        {
            return Err(HookCommandError);
        }
        Ok(Self {
            source,
            timeout,
            max_output_bytes,
        })
    }

    async fn execute(&self, request: CompactionHookRequest) -> CompactionHookResult {
        let failed = || CompactionHookResult::without_context(CompactionHookOutcome::Failed);
        let plan = match self.source.prepare(&request) {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                return CompactionHookResult::without_context(CompactionHookOutcome::Continue);
            }
            Err(_) => return failed(),
        };
        let Ok(input) = command_input(&request, &plan) else {
            return failed();
        };
        let limits = ProcessLimits {
            retained_bytes: self.max_output_bytes * 2,
            execution_timeout: self.timeout,
            drain_timeout: Duration::from_secs(1),
        };
        let launched = match plan.launcher {
            Some(launcher) => launcher.launch(&request, plan.process, limits).await,
            None => SupervisedProcess::spawn(plan.process, limits).map_err(|_| HookCommandError),
        };
        let Ok((process, stdin)) = launched else {
            return failed();
        };
        let Ok(output) = run_hook_process(process, stdin, input, false).await else {
            return failed();
        };
        if output.reason != StopReason::Completed
            || !output.drained
            || output.error.is_some()
            || output.resource_error.is_some()
            || output.stdout.omitted_bytes != 0
        {
            return failed();
        }
        let mut stdout = output.stdout.head;
        stdout.extend(output.stdout.tail);
        let Ok(stdout) = String::from_utf8(stdout) else {
            return failed();
        };
        CompactionHookResult::from_process_output(
            request.stage,
            output.exit.and_then(|exit| exit.code),
            &stdout,
            self.max_output_bytes,
        )
    }
}

/// One task owns stdin, output drains and child lifetime. Dropping this future
/// drops the process owner; a failed write requests cancellation and drains it.
pub(super) async fn run_hook_process(
    process: SupervisedProcess,
    mut stdin: PipeInput,
    input: Vec<u8>,
    allow_closed_stdin: bool,
) -> Result<ProcessOutcome, HookCommandError> {
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let finish = process.finish(async {
        let _ = stopped.await;
    });
    let write = async move {
        let mut offset = 0;
        while offset < input.len() {
            let count = match stdin.write(&input[offset..]).await {
                Ok(count) => count,
                Err(ProcessError::Io(std::io::ErrorKind::BrokenPipe)) if allow_closed_stdin => {
                    break;
                }
                Err(_) => return Err(HookCommandError),
            };
            if count == 0 {
                return Err(HookCommandError);
            }
            offset += count;
        }
        stdin.close();
        Ok::<_, HookCommandError>(())
    };
    tokio::pin!(write, finish);
    let (written, output) = tokio::select! {
        biased;
        result = &mut write => {
            if result.is_err() { let _ = stop.send(()); }
            (result, finish.await)
        }
        output = &mut finish => (if allow_closed_stdin { Ok(()) } else { Err(HookCommandError) }, output),
    };
    written.map(|_| output)
}

fn command_input(
    request: &CompactionHookRequest,
    plan: &AuthorizedHookCommand,
) -> Result<Vec<u8>, HookCommandError> {
    let event = match request.stage {
        CompactionHookStage::PreCompact => "PreCompact",
        CompactionHookStage::PostCompact => "PostCompact",
        CompactionHookStage::SessionStartCompact => "SessionStart",
    };
    let mut input = serde_json::json!({
        "session_id": request.conversation_id,
        "turn_id": request.turn_id,
        "operation_id": request.operation_id,
        "cwd": plan.process.directory,
        "transcript_path": plan.transcript_path,
        "hook_event_name": event,
        "model": request.model.model_id,
    });
    if request.stage == CompactionHookStage::SessionStartCompact {
        input["source"] = "compact".into();
    } else {
        input["trigger"] = match request.phase {
            CompactionPhase::Manual => "manual",
            _ => "auto",
        }
        .into();
    }
    let mut input = serde_json::to_vec(&input).map_err(|_| HookCommandError)?;
    if input.len() > 64 * 1024 {
        return Err(HookCommandError);
    }
    input.push(b'\n');
    Ok(input)
}

impl CompactionHooks for ProcessCompactionHooks {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async move { self.execute(request).await.outcome })
    }
    fn run_with_context(&self, request: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        Box::pin(self.execute(request))
    }
}

#[cfg(test)]
#[path = "compaction_hook_process/tests.rs"]
mod tests;
