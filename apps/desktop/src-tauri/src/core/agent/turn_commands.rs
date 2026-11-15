//! Turn-scoped cancellation. Acknowledgement is not a durable terminal event.
use super::{service::AgentSessionService, tasks::AgentTaskRegistry};
use research_agent_core::conversation::ConversationId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterruptRequest {
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    /// True means the matching live owner received a signal, not that it stopped.
    pub cancellation_requested: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InterruptError {
    Unavailable,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskObservationRequest {
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub run_id: Uuid,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskObservationResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub run_id: Uuid,
    pub status: TaskStatus,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TaskStatus {
    Pending,
    Settled,
    Finished {
        completion: &'static str,
        diagnostic: Option<research_agent_core::runner::ToolOutcomeDiagnostic>,
    },
    Unavailable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskObservationError {
    InvalidInput,
    Unavailable,
}

#[tauri::command]
pub async fn agent_observe_task(
    service: State<'_, Arc<AgentSessionService>>,
    request: TaskObservationRequest,
) -> Result<TaskObservationResponse, TaskObservationError> {
    observe_task_with_persistence(service.inner(), request).await
}

pub(crate) async fn observe_task_with_persistence(
    service: &AgentSessionService,
    request: TaskObservationRequest,
) -> Result<TaskObservationResponse, TaskObservationError> {
    let mut response = observe_task(service.tasks(), request).await?;
    if matches!(response.status, TaskStatus::Unavailable) {
        // A live receipt with this run ID but another turn must never be
        // replaced by a historical match, even under an ID collision.
        if service
            .tasks()
            .find_receipt(
                ConversationId::from_uuid(response.conversation_id),
                response.run_id,
            )
            .map_err(|_| TaskObservationError::Unavailable)?
            .is_some()
        {
            return Ok(response);
        }
        let database = service.task_settlement_database();
        if let Some(database) = database {
            let (conversation, turn, run) =
                (response.conversation_id, response.turn_id, response.run_id);
            let settled = tokio::task::spawn_blocking(move || {
                database.inspect_task_run_settlement(conversation, turn, run)
            })
            .await
            .map_err(|_| TaskObservationError::Unavailable)?
            .map_err(|_| TaskObservationError::Unavailable)?;
            if settled {
                response.status = TaskStatus::Settled;
            }
        }
    }
    Ok(response)
}

pub(crate) async fn observe_task(
    tasks: &AgentTaskRegistry,
    request: TaskObservationRequest,
) -> Result<TaskObservationResponse, TaskObservationError> {
    if request.conversation_id.is_nil() || request.turn_id.is_nil() || request.run_id.is_nil() {
        return Err(TaskObservationError::InvalidInput);
    }
    let receipt = tasks
        .find_receipt(
            ConversationId::from_uuid(request.conversation_id),
            request.run_id,
        )
        .map_err(|_| TaskObservationError::Unavailable)?;
    let status = match receipt.filter(|receipt| receipt.turn_id() == Some(request.turn_id)) {
        None => TaskStatus::Unavailable,
        Some(receipt) if !receipt.is_finished() => TaskStatus::Pending,
        Some(receipt) => {
            let result = receipt.observe().await;
            let completion = match result.completion {
                super::tasks::TaskCompletion::Completed => "completed",
                super::tasks::TaskCompletion::Canceled => "canceled",
                super::tasks::TaskCompletion::Failed => "failed",
                super::tasks::TaskCompletion::RecoveryRequired => "recoveryRequired",
            };
            TaskStatus::Finished {
                completion,
                diagnostic: result.diagnostic,
            }
        }
    };
    Ok(TaskObservationResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
        run_id: request.run_id,
        status,
    })
}

#[tauri::command]
pub async fn agent_interrupt_turn(
    service: State<'_, Arc<AgentSessionService>>,
    queues: State<'_, super::writing_submission::WritingSubmissionService>,
    request: InterruptRequest,
) -> Result<InterruptResponse, InterruptError> {
    let paused = queues
        .pause_queue_for_turn(request.conversation_id, request.turn_id)
        .await;
    finish_interrupt(service.tasks(), request, paused)
}
fn finish_interrupt(
    tasks: &AgentTaskRegistry,
    request: InterruptRequest,
    paused: Result<bool, super::repository::input_queue::Error>,
) -> Result<InterruptResponse, InterruptError> {
    // Even if persistence fails, deliver the stop signal. Report the failure so
    // the caller does not assume the durable queue has been paused.
    let response = interrupt(tasks, request);
    paused.map_err(|_| InterruptError::Unavailable)?;
    response
}

fn interrupt(
    tasks: &AgentTaskRegistry,
    request: InterruptRequest,
) -> Result<InterruptResponse, InterruptError> {
    let cancellation_requested = tasks
        .cancel_turn(
            ConversationId::from_uuid(request.conversation_id),
            request.turn_id,
        )
        .map_err(|_| InterruptError::Unavailable)?;
    Ok(InterruptResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
        cancellation_requested,
    })
}

#[cfg(test)]
#[path = "turn_commands/tests.rs"]
mod tests;
