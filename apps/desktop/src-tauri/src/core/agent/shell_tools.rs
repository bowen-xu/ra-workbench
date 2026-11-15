//! Shell tool orchestration over application-owned process sessions. This is not
//! a permission grant: a native host must authorize every operation and launch
//! only a sandboxed process in its frozen execution workspace.
use super::process::{ProcessPoll, ProcessRegistry, ProcessRegistryError};
use research_agent_core::{
    model::ToolCall,
    runner::{RunError, RunFuture, ToolExecutor, ToolOutcomeDiagnostic, ToolOutput},
    workspace::WorkspaceIdentity,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

#[path = "shell_tools/definitions.rs"]
mod definitions;
#[path = "shell_tools/output.rs"]
mod output;
#[cfg(target_os = "macos")]
#[path = "shell_tools/workspace_host.rs"]
pub mod workspace_host;
pub const EXEC: &str = "research_exec_command";
pub const PROCESS: &str = "research_process";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecRequest {
    pub command: String,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub yield_ms: u64,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessRequest {
    Poll { process_id: Uuid, yield_ms: u64 },
    Write { process_id: Uuid, text: String },
    CloseInput { process_id: Uuid },
    Cancel { process_id: Uuid },
}
#[derive(Clone)]
pub enum ShellRequest {
    Exec(ExecRequest),
    Process(ProcessRequest),
}
/// The native owner selects the lease; model arguments cannot change paths,
/// environment, shell program, sandbox policy or execution limits.
pub trait ShellHost: Send + Sync {
    /// Resolve the exact scope for a new call. A writing owner may prepare and
    /// select a successor only after the previous delivery is confirmed.
    fn scope(&self, original: WorkspaceIdentity) -> RunFuture<'_, WorkspaceIdentity> {
        Box::pin(async move { Ok(original) })
    }
    fn authorize(&self, scope: WorkspaceIdentity, request: ShellRequest) -> RunFuture<'_, ()>;
    /// Run blocking preparation/spawn on a blocking worker. Register the session
    /// in the supplied registry under exactly this scope. Failed/uncertain launch
    /// must preserve its persistent execution intent, never retry the command.
    fn launch(
        &self,
        registry: Arc<ProcessRegistry>,
        scope: WorkspaceIdentity,
        request: ExecRequest,
    ) -> RunFuture<'_, Uuid>;
}
struct Receipt {
    call: ToolCall,
    scope: WorkspaceIdentity,
    forget: Option<Uuid>,
    delivered: bool,
    acknowledged: bool,
}
pub struct ShellTools {
    registry: Arc<ProcessRegistry>,
    scope: WorkspaceIdentity,
    host: Arc<dyn ShellHost>,
    calls: Mutex<HashMap<String, Receipt>>,
}
impl ShellTools {
    fn receipt_error(
        &self,
        call: &ToolCall,
        scope: &WorkspaceIdentity,
        process: Option<Uuid>,
        cause: &'static str,
    ) -> RunError {
        let mut detail = serde_json::json!({
            "code":"SHELL_RESULT_ACK_UNCONFIRMED", "phase":"result-ack", "safeCause":cause,
            "conversationId":scope.conversation_id, "turnId":scope.turn_id,
            "leaseId":scope.lease_id,
        });
        // Opaque provider IDs are untrusted and must pass the diagnostic's
        // closed identifier validation; unsafe IDs must not suppress UUIDs.
        if !call.call_id.is_empty()
            && call.call_id.len() <= 1024
            && call
                .call_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-.:/".contains(&byte))
        {
            detail["callId"] = call.call_id.clone().into();
        }
        if let Some(process) = process {
            detail["resultId"] = serde_json::json!(process);
        }
        ToolOutcomeDiagnostic::try_from_value(detail)
            .map(|detail| RunError::EventSinkDiagnostic(Box::new(detail)))
            .unwrap_or(RunError::EventSink)
    }

    pub fn new(
        registry: Arc<ProcessRegistry>,
        scope: WorkspaceIdentity,
        host: Arc<dyn ShellHost>,
    ) -> Self {
        Self {
            registry,
            scope,
            host,
            calls: Mutex::new(HashMap::new()),
        }
    }
    fn parse(call: &ToolCall) -> Result<ShellRequest, RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 {
            return Err(RunError::ToolRejected);
        }
        let request = match call.name.as_str() {
            EXEC => ShellRequest::Exec(crate::agent::tool_arguments::decode_object(
                &call.arguments,
                "Provide the fields for this shell operation with the types declared in its tool parameters.",
            )?),
            PROCESS => ShellRequest::Process(crate::agent::tool_arguments::decode_object(
                &call.arguments,
                "Provide the fields for this shell operation with the types declared in its tool parameters.",
            )?),
            _ => return Err(RunError::ToolRejected),
        };
        let valid = match &request {
            ShellRequest::Exec(args) => {
                !args.command.is_empty()
                    && args.command.len() <= 64 * 1024
                    && !args.command.contains('\0')
                    && args.yield_ms <= 60_000
            }
            ShellRequest::Process(args) => match args {
                ProcessRequest::Poll {
                    process_id,
                    yield_ms,
                } => !process_id.is_nil() && *yield_ms <= 60_000,
                ProcessRequest::Write { process_id, text } => {
                    !process_id.is_nil() && text.len() <= 64 * 1024
                }
                ProcessRequest::CloseInput { process_id }
                | ProcessRequest::Cancel { process_id } => !process_id.is_nil(),
            },
        };
        if valid {
            Ok(request)
        } else {
            Err(RunError::ToolArguments(
                "Shell arguments exceed the declared command, process identity or yield limits."
                    .into(),
            ))
        }
    }
    async fn poll(
        &self,
        scope: &WorkspaceIdentity,
        id: Uuid,
        yield_ms: u64,
    ) -> Result<(ToolOutput, Option<Uuid>), RunError> {
        let poll = self
            .registry
            .poll(scope, id, Duration::from_millis(yield_ms))
            .await
            .map_err(|_| RunError::EventSink)?;
        let forget = match &poll {
            ProcessPoll::Finished(outcome)
                if outcome.drained && outcome.resource_error.is_none() =>
            {
                Some(id)
            }
            _ => None,
        };
        Ok((output::render(id, &poll)?, forget))
    }
}
impl ToolExecutor for ShellTools {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        Self::parse(call).map(|_| ())
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            let request = Self::parse(&call)?;
            {
                let calls = self.calls.lock().map_err(|_| RunError::EventSink)?;
                if calls.contains_key(&call.call_id) || calls.len() >= 4096 {
                    return Err(RunError::ToolRejected);
                }
            }
            let scope = self.host.scope(self.scope.clone()).await?;
            if scope.conversation_id != self.scope.conversation_id
                || scope.turn_id != self.scope.turn_id
                || scope.environment_id != self.scope.environment_id
                || scope.lease_id.is_nil()
            {
                return Err(RunError::ToolRejected);
            }
            self.host.authorize(scope.clone(), request.clone()).await?;
            {
                let mut calls = self.calls.lock().map_err(|_| RunError::EventSink)?;
                // Retain call identities for this turn, including failed or
                // acknowledged calls: retrying stdin or exec can duplicate effects.
                if calls.contains_key(&call.call_id) || calls.len() >= 4096 {
                    return Err(RunError::ToolRejected);
                }
                calls.insert(
                    call.call_id.clone(),
                    Receipt {
                        call: call.clone(),
                        scope: scope.clone(),
                        forget: None,
                        delivered: false,
                        acknowledged: false,
                    },
                );
            }
            let (result, forget) = match request {
                ShellRequest::Exec(args) => {
                    let yield_ms = args.yield_ms;
                    let id = self
                        .host
                        .launch(self.registry.clone(), scope.clone(), args)
                        .await?;
                    self.poll(&scope, id, yield_ms).await?
                }
                ShellRequest::Process(ProcessRequest::Poll {
                    process_id,
                    yield_ms,
                }) => self.poll(&scope, process_id, yield_ms).await?,
                ShellRequest::Process(ProcessRequest::Write { process_id, text }) => {
                    let written = self
                        .registry
                        .write(&scope, process_id, text.as_bytes())
                        .await
                        .map_err(|_| RunError::EventSink)?;
                    (
                        ToolOutput {
                            content:
                                serde_json::json!({"processId":process_id,"writtenBytes":written,
                        "requestedBytes":text.len()})
                                .to_string(),
                            is_error: false,
                        },
                        None,
                    )
                }
                ShellRequest::Process(ProcessRequest::CloseInput { process_id }) => {
                    self.registry
                        .close_input(&scope, process_id)
                        .await
                        .map_err(|_| RunError::EventSink)?;
                    (
                        ToolOutput {
                            content: "Input handle closed; for a terminal this does not send EOF."
                                .into(),
                            is_error: false,
                        },
                        None,
                    )
                }
                ShellRequest::Process(ProcessRequest::Cancel { process_id }) => {
                    self.registry
                        .cancel(&scope, process_id)
                        .map_err(|_| RunError::EventSink)?;
                    (ToolOutput { content: "Cancellation requested; poll to confirm exit and collect final output.".into(), is_error: false }, None)
                }
            };
            let mut calls = self.calls.lock().map_err(|_| RunError::EventSink)?;
            let receipt = calls.get_mut(&call.call_id).ok_or(RunError::EventSink)?;
            receipt.forget = forget;
            receipt.delivered = true;
            Ok(result)
        })
    }
    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async move {
            let mut calls = self.calls.lock().map_err(|_| {
                self.receipt_error(&call, &self.scope, None, "receipt_mutex_poisoned")
            })?;
            let receipt = calls
                .get_mut(&call.call_id)
                .ok_or_else(|| self.receipt_error(&call, &self.scope, None, "receipt_missing"))?;
            if !receipt.delivered
                || receipt.call.name != call.name
                || receipt.call.arguments != call.arguments
            {
                return Err(self.receipt_error(
                    &call,
                    &receipt.scope,
                    receipt.forget,
                    "receipt_identity_mismatch",
                ));
            }
            if !receipt.acknowledged {
                if let Some(id) = receipt.forget {
                    self.registry.forget(&receipt.scope, id).map_err(|error| {
                        self.receipt_error(
                            &call,
                            &receipt.scope,
                            Some(id),
                            match error {
                                ProcessRegistryError::NotFound => "process_not_found",
                                ProcessRegistryError::Busy => "process_not_drained",
                                _ => "process_registry_unavailable",
                            },
                        )
                    })?;
                }
                receipt.acknowledged = true;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "shell_tools/tests.rs"]
mod tests;
