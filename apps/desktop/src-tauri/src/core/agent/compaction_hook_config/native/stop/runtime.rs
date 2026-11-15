//! Task-owned composition of Stop dispatch, committed audit and continuation.
use super::{NativeStopHandlers, StopRecordedError, StopResolution, commit};
use crate::agent::{
    cancellation::AgentCancellation,
    repository::stop_hook_audit::SqliteStopHookAudit,
    service::AgentSessionService,
    stop_hook_process::{StopRequest, StopTarget},
};
use research_agent_core::{
    model::{ModelMessage, ModelRequest, ModelResponse},
    runner::{
        RunError, RunFuture, StopHookOutcome, StopHooks, ToolOutcomeDiagnostic,
        stop_hook_last_assistant_message,
    },
    submission::SubmissionRecord,
};
use uuid::Uuid;

#[allow(
    dead_code,
    reason = "the runner adapter will consume the outcome fields"
)]
pub(crate) enum NativeStopOutcome {
    Continue,
    Stop { reason: Option<String> },
    FollowUp(ModelMessage),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum NativeStopError {
    #[error("stop hook task identity or data owner is invalid")]
    InvalidOwner,
    #[error("stop hook task canceled")]
    Canceled,
    #[error(transparent)]
    Dispatch(#[from] StopRecordedError),
    #[error(transparent)]
    Feedback(#[from] commit::StopPromptCommitError),
}

/// The existing task owns this future through audit completion and journal
/// admission. No handler or continuation is detached or automatically retried.
#[allow(
    dead_code,
    reason = "the production sampling context will own this adapter"
)]
pub(crate) struct NativeStopRuntime<'a> {
    pub handlers: &'a NativeStopHandlers,
    pub service: &'a AgentSessionService,
    pub audit: &'a SqliteStopHookAudit,
    pub record: &'a SubmissionRecord,
    pub signal: &'a AgentCancellation,
    pub permission_mode: &'a str,
    pub target: StopTarget,
}
impl NativeStopRuntime<'_> {
    #[allow(dead_code, reason = "the production sampling context will invoke Stop")]
    pub async fn run(
        &self,
        turn_id: Uuid,
        request: &ModelRequest,
        response: &ModelResponse,
        stop_hook_active: bool,
    ) -> Result<NativeStopOutcome, NativeStopError> {
        if !self.audit.belongs_to(self.service)
            || turn_id != self.record.turn_id
            || request.model != self.record.input.model
            || request.request_id.is_nil()
        {
            return Err(NativeStopError::InvalidOwner);
        }
        if self.signal.is_cancelled() {
            return Err(NativeStopError::Canceled);
        }
        let input = StopRequest {
            conversation_id: self.record.conversation_id,
            turn_id,
            model: self.record.input.model.clone(),
            permission_mode: self.permission_mode.into(),
            stop_hook_active,
            last_assistant_message: stop_hook_last_assistant_message(response),
            target: self.target.clone(),
        };
        let Some(result) = self
            .handlers
            .run_recorded(&input, self.audit, self.signal.cancelled())
            .await?
        else {
            if self.signal.is_cancelled() {
                return Err(NativeStopError::Canceled);
            }
            return Ok(NativeStopOutcome::Continue);
        };
        if self.signal.is_cancelled() || result.canceled {
            return Err(NativeStopError::Canceled);
        }
        match result.resolution() {
            StopResolution::Canceled => Err(NativeStopError::Canceled),
            StopResolution::Stop { reason } => Ok(NativeStopOutcome::Stop { reason }),
            StopResolution::Continue => Ok(NativeStopOutcome::Continue),
            StopResolution::FollowUp { .. } => {
                let message = commit::persist_follow_up(self.service, self.record, &result).await?;
                if self.signal.is_cancelled() {
                    return Err(NativeStopError::Canceled);
                }
                Ok(NativeStopOutcome::FollowUp(message))
            }
        }
    }
}

impl StopHooks for NativeStopRuntime<'_> {
    fn run<'a>(
        &'a self,
        turn_id: Uuid,
        request: &'a ModelRequest,
        response: &'a ModelResponse,
        stop_hook_active: bool,
    ) -> RunFuture<'a, StopHookOutcome> {
        Box::pin(async move {
            match NativeStopRuntime::run(self, turn_id, request, response, stop_hook_active).await {
                Ok(NativeStopOutcome::Continue) => Ok(StopHookOutcome::Continue),
                Ok(NativeStopOutcome::Stop { .. }) => Ok(StopHookOutcome::Stop),
                Ok(NativeStopOutcome::FollowUp(message)) => Ok(StopHookOutcome::FollowUp(message)),
                Err(NativeStopError::Canceled) => {
                    Err(research_agent_core::model::ModelError::Canceled.into())
                }
                Err(error) => Err(self.runner_error(error)),
            }
        })
    }
}

impl NativeStopRuntime<'_> {
    fn runner_error(&self, error: NativeStopError) -> RunError {
        let sqlite_code = match &error {
            NativeStopError::Dispatch(StopRecordedError::Storage {
                source: rusqlite::Error::SqliteFailure(code, _),
                ..
            }) => Some(code.extended_code),
            _ => None,
        };
        let item_id = match &error {
            NativeStopError::Feedback(commit::StopPromptCommitError::Commit {
                item_id, ..
            }) => Some(*item_id),
            _ => None,
        };
        let (phase, cause, operation) = match error {
            NativeStopError::InvalidOwner => ("validate", "invalid_owner", None),
            NativeStopError::Canceled => {
                return research_agent_core::model::ModelError::Canceled.into();
            }
            NativeStopError::Dispatch(StopRecordedError::Storage { invocation_id, .. }) => {
                ("audit", "storage_failure", Some(invocation_id))
            }
            NativeStopError::Dispatch(StopRecordedError::Dispatch { invocation_id }) => {
                ("dispatch", "dispatch_failure", invocation_id)
            }
            NativeStopError::Dispatch(StopRecordedError::Clock(invocation_id)) => {
                ("audit", "clock_unavailable", Some(invocation_id))
            }
            NativeStopError::Feedback(commit::StopPromptCommitError::Unmanaged(id)) => {
                ("feedback", "unmanaged_store", Some(id))
            }
            NativeStopError::Feedback(commit::StopPromptCommitError::NoFollowUp(id)) => {
                ("feedback", "missing_follow_up", Some(id))
            }
            NativeStopError::Feedback(commit::StopPromptCommitError::Commit {
                invocation_id,
                ..
            }) => ("feedback", "commit_failure", Some(invocation_id)),
        };
        let mut value = serde_json::json!({
            "code": "stop_hook_failure", "phase": phase, "safeCause": cause,
            "conversationId": self.record.conversation_id, "turnId": self.record.turn_id,
            "requestId": self.record.request_id,
        });
        if let Some(id) = operation {
            value["operationId"] = serde_json::json!(id);
        }
        if let Some(code) = sqlite_code {
            value["sqliteExtendedCode"] = serde_json::json!(code);
        }
        if let Some(id) = item_id {
            value["resultId"] = serde_json::json!(id);
        }
        ToolOutcomeDiagnostic::try_from_value(value)
            .map(|diagnostic| RunError::EventSinkDiagnostic(Box::new(diagnostic)))
            .unwrap_or(RunError::EventSink)
    }
}
