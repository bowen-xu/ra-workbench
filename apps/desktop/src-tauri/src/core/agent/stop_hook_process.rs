//! Native Stop/SubagentStop process adapter for the shared output decoder.
//! Input protocol adapted from Codex hooks/src/events/stop.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b (Apache-2.0, Copyright 2025 OpenAI).
//! The caller owns handler trust, audit, cancellation and durable feedback.
use super::{
    compaction_hook_process::{HookCommandError, HookLaunchFuture, run_hook_process},
    process::{ProcessLimits, ProcessSpec, RetainedOutput, StopReason},
};
use research_agent_core::{
    protocol::ModelIdentity,
    runner::{StopEvent, StopHandlerOutput, StopHandlerStatus},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Clone, PartialEq, Eq)]
pub enum StopTarget {
    Stop,
    SubagentStop {
        agent_id: String,
        agent_type: String,
        agent_transcript_path: Option<PathBuf>,
    },
}

#[derive(Clone)]
pub struct StopRequest {
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub model: ModelIdentity,
    pub permission_mode: String,
    pub stop_hook_active: bool,
    pub last_assistant_message: Option<String>,
    pub target: StopTarget,
}

/// The host must revalidate its frozen source, command, workspace and grants
/// immediately before launch, and return the supervised owner with any leases.
pub trait StopHookLauncher: Send + Sync {
    fn launch<'a>(
        &'a self,
        request: &'a StopRequest,
        process: ProcessSpec,
        limits: ProcessLimits,
    ) -> HookLaunchFuture<'a>;
}

pub struct AuthorizedStopCommand {
    pub process: ProcessSpec,
    pub transcript_path: Option<PathBuf>,
    pub launcher: Arc<dyn StopHookLauncher>,
}

/// Native-only frozen command resolution; never deserialize from model or IPC.
pub trait StopCommandSource: Send + Sync {
    fn prepare(
        &self,
        request: &StopRequest,
    ) -> Result<Option<AuthorizedStopCommand>, HookCommandError>;
}

pub struct ProcessStopHook {
    source: Arc<dyn StopCommandSource>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl ProcessStopHook {
    pub fn new(
        source: Arc<dyn StopCommandSource>,
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

    pub async fn run(&self, request: &StopRequest) -> StopHandlerOutput {
        let plan = match self.source.prepare(request) {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                return StopHandlerOutput {
                    status: StopHandlerStatus::Completed,
                    system_message: None,
                };
            }
            Err(_) => return failed("hook command authorization failed"),
        };
        let input = match command_input(request, &plan) {
            Ok(input) => input,
            Err(_) => return failed("hook input serialization failed"),
        };
        let limits = ProcessLimits {
            retained_bytes: self.max_output_bytes * 2,
            execution_timeout: self.timeout,
            drain_timeout: Duration::from_secs(1),
        };
        let (process, stdin) = match plan.launcher.launch(request, plan.process, limits).await {
            Ok(launched) => launched,
            Err(_) => return failed("hook process launch failed"),
        };
        let output = match run_hook_process(process, stdin, input, true).await {
            Ok(output) => output,
            Err(_) => return failed("hook process input failed"),
        };
        if output.reason != StopReason::Completed
            || !output.drained
            || output.error.is_some()
            || output.resource_error.is_some()
        {
            return failed("hook process did not complete safely");
        }
        let code = output.exit.and_then(|exit| exit.code);
        // Codex reads stdout only on exit 0, stderr only on exit 2. An
        // irrelevant stream cannot invalidate otherwise usable feedback.
        let (stdout, stderr) = match code {
            Some(0) => match complete_text(output.stdout) {
                Some(text) => (text, String::new()),
                None => return failed("hook stdout is incomplete"),
            },
            Some(2) => match complete_text(output.stderr) {
                Some(text) => (String::new(), text),
                None => return failed("hook stderr is incomplete"),
            },
            _ => (String::new(), String::new()),
        };
        let event = match request.target {
            StopTarget::Stop => StopEvent::Stop,
            StopTarget::SubagentStop { .. } => StopEvent::SubagentStop,
        };
        StopHandlerOutput::from_process_output(event, code, &stdout, &stderr)
    }
}

fn complete_text(output: RetainedOutput) -> Option<String> {
    if output.omitted_bytes != 0 {
        return None;
    }
    let mut bytes = output.head;
    bytes.extend(output.tail);
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn failed(diagnostic: &str) -> StopHandlerOutput {
    StopHandlerOutput {
        status: StopHandlerStatus::Failed {
            diagnostic: diagnostic.into(),
        },
        system_message: None,
    }
}

fn command_input(
    request: &StopRequest,
    plan: &AuthorizedStopCommand,
) -> Result<Vec<u8>, HookCommandError> {
    let event = match request.target {
        StopTarget::Stop => "Stop",
        StopTarget::SubagentStop { .. } => "SubagentStop",
    };
    let cwd = plan.process.directory.display().to_string();
    let transcript_path = plan
        .transcript_path
        .as_ref()
        .map(|path| path.display().to_string());
    let mut input = serde_json::json!({
        "session_id": request.conversation_id,
        "turn_id": request.turn_id,
        "transcript_path": transcript_path,
        "cwd": cwd,
        "hook_event_name": event,
        "model": request.model.model_id,
        "permission_mode": request.permission_mode,
        "stop_hook_active": request.stop_hook_active,
        "last_assistant_message": request.last_assistant_message,
    });
    if let StopTarget::SubagentStop {
        agent_id,
        agent_type,
        agent_transcript_path,
    } = &request.target
    {
        input["agent_id"] = agent_id.as_str().into();
        input["agent_type"] = agent_type.as_str().into();
        input["agent_transcript_path"] = serde_json::to_value(
            agent_transcript_path
                .as_ref()
                .map(|path| path.display().to_string()),
        )
        .map_err(|_| HookCommandError)?;
    }
    let mut bytes = serde_json::to_vec(&input).map_err(|_| HookCommandError)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
#[path = "stop_hook_process/tests.rs"]
mod tests;
