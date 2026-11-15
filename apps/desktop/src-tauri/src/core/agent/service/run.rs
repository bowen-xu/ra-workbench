//! Keep task completion and durable turn reconciliation under the same owner.
use super::*;
use crate::agent::{
    cancellation::AgentCancellation,
    tasks::{TaskCompletion, TaskObservation, TaskReceipt, TaskRegistryError},
};
use research_agent_core::runner::RunError;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct OwnedTurn {
    pub turn_id: Uuid,
    pub task: TaskReceipt,
}

impl AgentSessionService {
    /// Submit native work after creating/opening the conversation. The operation
    /// must commit its input using the supplied fresh turn ID, then run the runner.
    /// The registry owns setup, execution and reconciliation even if the UI leaves.
    pub fn start_owned_turn<F, Fut>(
        self: &Arc<Self>,
        id: ConversationId,
        operation: F,
    ) -> Result<OwnedTurn, TaskRegistryError>
    where
        F: FnOnce(Uuid, AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        let turn_id = Uuid::new_v4();
        self.start_turn_with_id(
            id,
            turn_id,
            Arc::new(AtomicBool::new(false)),
            None,
            operation,
        )
    }

    pub(super) fn start_turn_with_id<F, Fut>(
        self: &Arc<Self>,
        id: ConversationId,
        turn_id: Uuid,
        turn_owned: Arc<AtomicBool>,
        settlement: Option<crate::data_interface::TaskSettlement>,
        operation: F,
    ) -> Result<OwnedTurn, TaskRegistryError>
    where
        F: FnOnce(Uuid, AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        let service = self.clone();
        let run_id = Uuid::new_v4();
        let completion_claim = turn_owned.clone();
        #[cfg(unix)]
        let execution_service = self.clone();
        let task = self.tasks.start_with_cleanup_claim_id(
            id,
            Some(turn_id),
            turn_owned,
            run_id,
            move |signal| async move {
                #[cfg(unix)]
                execution_service
                    .processes
                    .begin_turn(*id.as_uuid(), turn_id)
                    .map_err(|error| match error {
                        crate::agent::process::ProcessRegistryError::Closing => {
                            RunError::Model(research_agent_core::model::ModelError::Canceled)
                        }
                        _ => RunError::EventSink,
                    })?;
                #[cfg(unix)]
                {
                    let cancellation = signal.clone();
                    let running = operation(turn_id, signal);
                    tokio::pin!(running);
                    tokio::select! {
                        result = &mut running => result,
                        _ = cancellation.cancelled() => {
                            // Do not join while the paused operation might own a
                            // process lock. Signal now, keep polling durable work.
                            let failed = execution_service.processes.stop_turn(*id.as_uuid(), turn_id).is_err();
                            let result = running.await;
                            if failed { Err(RunError::EventSink) } else { result }
                        }
                    }
                }
                #[cfg(not(unix))]
                operation(turn_id, signal).await
            },
            move |completion| async move {
                #[cfg(unix)]
                let process_failed = service
                    .processes
                    .finish_turn(*id.as_uuid(), turn_id)
                    .await
                    .is_err();
                #[cfg(not(unix))]
                let process_failed = false;
                let result = match service.reconcile_stopped_turn(id, turn_id).await {
                    Ok(false) if !process_failed => completion,
                    Ok(false) => TaskCompletion::RecoveryRequired,
                    // A missing terminal is abnormal even if work returned Ok.
                    Ok(true) | Err(_) => TaskCompletion::RecoveryRequired,
                };
                if result == TaskCompletion::RecoveryRequired || !completion_claim.load(Ordering::Acquire) {
                    return TaskObservation { completion: result, diagnostic: None };
                }
                if let (Some(task), Some(db)) = (settlement, service.compaction_db.clone().filter(|db| db.logged())) {
                    let identity = task.clone();
                    let saved = tokio::task::spawn_blocking(move || db.record_task_settlement(&task, run_id)).await;
                    match saved {
                        Ok(Ok(_)) => {},
                        Ok(Err(error)) => return TaskObservation { completion: TaskCompletion::RecoveryRequired,
                            diagnostic: settlement_diagnostic(&identity, &error) },
                        Err(_) => return TaskObservation { completion: TaskCompletion::RecoveryRequired, diagnostic: None },
                    }
                }
                TaskObservation { completion: result, diagnostic: None }
            },
        )?;
        Ok(OwnedTurn { turn_id, task })
    }

    /// Only called after the owned operation has joined, while its task still
    /// reserves admission. Never rerun external work or terminate a different turn.
    async fn reconcile_stopped_turn(
        &self,
        id: ConversationId,
        turn_id: Uuid,
    ) -> Result<bool, AgentServiceError> {
        let slot = match self.slot(id, false) {
            Ok(slot) => slot,
            Err(AgentServiceError::NotOpen) => return Ok(false),
            Err(error) => return Err(error),
        };
        let mut owner = slot.lock().await;
        if owner
            .as_ref()
            .is_some_and(DurableConversation::needs_recovery)
        {
            owner.take();
            // Install before further awaits so interruption/error leaves a live
            // recovery barrier instead of discarding an unacknowledged commit.
            *owner = Some(DurableConversation::resume(self.store.clone(), fresh_lease(id)).await?);
        }
        let Some(session) = owner.as_mut() else {
            return Ok(false);
        };
        match session.projection().active_turn() {
            None => Ok(false),
            Some(active) if active != turn_id => Err(AgentServiceError::Active),
            Some(_) => {
                if let Some(receipt) = session
                    .interrupt_recovered_turn(Uuid::new_v4(), now_ms()?)
                    .await?
                {
                    self.notify(id, &receipt);
                }
                Ok(true)
            }
        }
    }
}

fn settlement_diagnostic(
    task: &crate::data_interface::TaskSettlement,
    error: &crate::data_interface::SqlImmediateFailure,
) -> Option<research_agent_core::runner::ToolOutcomeDiagnostic> {
    let detail = error.diagnostic()?;
    research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(serde_json::json!({
        "code": error.code(), "phase": detail.phase, "safeCause": detail.safe_cause,
        "conversationId": task.conversation_id, "requestId": task.request_id,
        "turnId": task.turn_id, "operationId": detail.operation_id,
        "sqliteCode": detail.sqlite_code, "sqliteExtendedCode": detail.sqlite_extended_code,
    }))
    .ok()
}

#[cfg(test)]
#[path = "run/tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "run/process_tests.rs"]
mod process_tests;
