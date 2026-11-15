//! Application-owned tasks, independent of UI subscriptions.
//! Based on Codex's RunningTask ownership boundary; unlike AbortOnDropHandle,
//! graceful cancellation preserves in-flight durable event commits.
use super::cancellation::AgentCancellation;
use research_agent_core::{
    conversation::ConversationId,
    model::ModelError,
    runner::{RunError, ToolOutcomeDiagnostic},
    subtasks::{self, SubtaskSpec},
};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::Mutex as AsyncMutex,
    task::{AbortHandle, JoinHandle},
};
use uuid::Uuid;
#[path = "tasks/reservation.rs"]
mod reservation;

const MAX_TASKS: usize = 128;
const MAX_RECENT_RECEIPTS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskCompletion {
    Completed,
    Canceled,
    Failed,
    /// Persistence failed, a tool effect is unknown, or the task panicked.
    /// Durable history must be reconciled.
    RecoveryRequired,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TaskRegistryError {
    #[error("agent task registry unavailable")]
    Unavailable,
    #[error("agent service is shutting down")]
    Closing,
    #[error("conversation already has an active task")]
    Active,
    #[error("agent task capacity reached")]
    Capacity,
    #[error("subtask scope is invalid")]
    SubtaskInvalid,
}

struct TaskJoin {
    handle: Option<JoinHandle<TaskObservation>>,
    result: Option<TaskObservation>,
}

/// In-process observation of the original task after its owner has reconciled.
#[derive(Clone, Debug)]
pub struct TaskObservation {
    pub completion: TaskCompletion,
    pub diagnostic: Option<ToolOutcomeDiagnostic>,
}
struct TaskEntry {
    run_id: Uuid,
    turn_id: Option<Uuid>,
    turn_owned: Arc<AtomicBool>,
    cancellation: AgentCancellation,
    // Read completion without taking the waiter's join lock; never abort it.
    completion: AbortHandle,
    join: AsyncMutex<TaskJoin>,
}
impl TaskEntry {
    fn finished(&self) -> bool {
        self.completion.is_finished()
    }
    async fn wait(&self) -> TaskObservation {
        let mut task = self.join.lock().await;
        if let Some(result) = &task.result {
            return result.clone();
        }
        // Await by reference: canceling this waiter retains the handle in its owner.
        let result = match task.handle.as_mut() {
            Some(handle) => handle.await.unwrap_or(TaskObservation {
                completion: TaskCompletion::RecoveryRequired,
                diagnostic: None,
            }),
            None => TaskObservation {
                completion: TaskCompletion::RecoveryRequired,
                diagnostic: None,
            },
        };
        task.handle = None;
        task.result = Some(result.clone());
        result
    }
}

/// Dropping a receipt does not stop or detach its task from the registry.
#[derive(Clone)]
pub struct TaskReceipt(Arc<TaskEntry>);
impl TaskReceipt {
    pub fn run_id(&self) -> Uuid {
        self.0.run_id
    }
    pub fn turn_id(&self) -> Option<Uuid> {
        self.0.turn_id
    }
    pub fn is_finished(&self) -> bool {
        self.0.finished()
    }
    pub async fn wait(&self) -> TaskCompletion {
        self.0.wait().await.completion
    }
    pub async fn observe(&self) -> TaskObservation {
        self.0.wait().await
    }
}
#[derive(Default)]
struct RegistryState {
    closing: bool,
    tasks: HashMap<ConversationId, Arc<TaskEntry>>,
    recent: VecDeque<(ConversationId, Arc<TaskEntry>)>,
    reservations: std::collections::HashSet<ConversationId>,
    batch_reservations: std::collections::HashSet<ConversationId>,
    children: HashMap<ConversationId, std::collections::HashSet<ConversationId>>,
}
#[derive(Default)]
pub struct AgentTaskRegistry {
    state: Mutex<RegistryState>,
    reservation_changed: tokio::sync::Notify,
}
impl AgentTaskRegistry {
    fn retain_finished(state: &mut RegistryState) {
        let mut finished = Vec::new();
        state.tasks.retain(|id, task| {
            if task.finished() {
                finished.push((*id, task.clone()));
                false
            } else {
                true
            }
        });
        for entry in finished {
            if state.recent.len() == MAX_RECENT_RECEIPTS {
                state.recent.pop_front();
            }
            state.recent.push_back(entry);
        }
    }

    /// Locate only the specified in-process owner. Missing or evicted receipts
    /// carry no implication about durable history or whether work occurred.
    pub fn find_receipt(
        &self,
        conversation_id: ConversationId,
        run_id: Uuid,
    ) -> Result<Option<TaskReceipt>, TaskRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        Ok(state
            .tasks
            .get(&conversation_id)
            .filter(|task| task.run_id == run_id)
            .or_else(|| {
                state.recent.iter().find_map(|(id, task)| {
                    (*id == conversation_id && task.run_id == run_id).then_some(task)
                })
            })
            .cloned()
            .map(TaskReceipt))
    }

    /// Find the sole retained owner for a proven conversation and turn.
    /// Ambiguity or absence cannot establish an execution outcome.
    pub(crate) fn find_turn_receipt(
        &self,
        conversation_id: ConversationId,
        turn_id: Uuid,
    ) -> Result<Option<TaskReceipt>, TaskRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        let mut matches = state
            .tasks
            .get(&conversation_id)
            .into_iter()
            .chain(
                state
                    .recent
                    .iter()
                    .filter_map(|(id, task)| (*id == conversation_id).then_some(task)),
            )
            .filter(|task| {
                task.turn_id == Some(turn_id) && task.turn_owned.load(Ordering::Acquire)
            });
        let Some(first) = matches.next() else {
            return Ok(None);
        };
        if matches.next().is_some() {
            return Ok(None);
        }
        Ok(Some(TaskReceipt(first.clone())))
    }

    fn prune_lineage(state: &mut RegistryState) {
        let active = state
            .tasks
            .iter()
            .filter_map(|(conversation, task)| (!task.finished()).then_some(*conversation))
            .collect::<std::collections::HashSet<_>>();
        state.children.retain(|parent, children| {
            if !active.contains(parent) {
                return false;
            }
            children.retain(|child| active.contains(child));
            !children.is_empty()
        });
    }

    fn reaches(state: &RegistryState, start: ConversationId, target: ConversationId) -> bool {
        let mut pending = state.children.get(&start).cloned().unwrap_or_default();
        while let Some(next) = pending.iter().next().copied() {
            pending.remove(&next);
            if next == target {
                return true;
            }
            if let Some(descendants) = state.children.get(&next) {
                pending.extend(descendants.iter().copied());
            }
        }
        false
    }

    fn cancel_descendants(state: &RegistryState, conversation_id: ConversationId) {
        let mut pending = state
            .children
            .get(&conversation_id)
            .cloned()
            .unwrap_or_default();
        while let Some(child) = pending.iter().next().copied() {
            pending.remove(&child);
            if let Some(entry) = state.tasks.get(&child) {
                entry.cancellation.cancel();
            }
            if let Some(descendants) = state.children.get(&child) {
                pending.extend(descendants.iter().copied());
            }
        }
    }

    /// Returns whether a conversation currently owns a live task. This is used
    /// before a host creates a child so a failed parent admission cannot leave
    /// an orphaned child edge.
    pub fn has_active(&self, conversation_id: ConversationId) -> Result<bool, TaskRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        Ok(state
            .tasks
            .get(&conversation_id)
            .is_some_and(|task| !task.finished()))
    }

    /// Link two already-admitted task owners. Child execution is still started
    /// by the normal submission path; this method only transfers cancellation
    /// lineage, avoiding a second owner for the same child conversation.
    pub fn link_child(
        &self,
        parent: ConversationId,
        child: ConversationId,
    ) -> Result<(), TaskRegistryError> {
        if parent == child {
            return Err(TaskRegistryError::SubtaskInvalid);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        if state.tasks.get(&parent).is_none_or(|task| task.finished())
            || !state.tasks.contains_key(&child)
        {
            return Err(TaskRegistryError::SubtaskInvalid);
        }
        // Adding parent -> child is invalid when child already reaches parent;
        // otherwise recursive cancellation would never have a finite traversal.
        if Self::reaches(&state, child, parent) {
            return Err(TaskRegistryError::SubtaskInvalid);
        }
        state.children.entry(parent).or_default().insert(child);
        // Cancellation may have been requested after the child was admitted
        // but before this edge existed. Both operations hold this same lock.
        if state
            .tasks
            .get(&parent)
            .is_some_and(|task| task.cancellation.is_cancelled())
        {
            Self::cancel_descendants(&state, parent);
        }
        Ok(())
    }

    /// The operation includes durable turn/input setup and then run_cancellable_turn.
    /// It must honor the signal at external-work boundaries, never abort durable IO.
    /// This native entry point is not an IPC tool or an authorization bypass.
    pub fn start<F, Fut>(
        &self,
        conversation_id: ConversationId,
        operation: F,
    ) -> Result<TaskReceipt, TaskRegistryError>
    where
        F: FnOnce(AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        self.start_with_cleanup(
            conversation_id,
            None,
            operation,
            |result| async move { result },
        )
    }

    /// Start a bounded child conversation while retaining the parent/child
    /// ownership relation. Cancellation of the parent is propagated to all
    /// registered descendants; the child still reconciles its own durable IO.
    pub fn start_child<F, Fut>(
        &self,
        parent: &SubtaskSpec,
        child: &SubtaskSpec,
        operation: F,
    ) -> Result<TaskReceipt, TaskRegistryError>
    where
        F: FnOnce(AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        self.start_child_with_cleanup(parent, child, operation, |result| async move { result })
    }

    pub fn start_child_with_cleanup<F, Fut, C, Cleanup>(
        &self,
        parent: &SubtaskSpec,
        child: &SubtaskSpec,
        operation: F,
        cleanup: C,
    ) -> Result<TaskReceipt, TaskRegistryError>
    where
        F: FnOnce(AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
        C: FnOnce(TaskCompletion) -> Cleanup + Send + 'static,
        Cleanup: Future<Output = TaskCompletion> + Send + 'static,
    {
        subtasks::validate_child(parent, child).map_err(|_| TaskRegistryError::SubtaskInvalid)?;
        let parent_id = ConversationId::from_uuid(child.parent_conversation_id);
        let child_id = ConversationId::from_uuid(child.child_conversation_id);
        let parent_run_id = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?
            .tasks
            .get(&parent_id)
            .filter(|task| !task.finished())
            .map(|task| task.run_id)
            .ok_or(TaskRegistryError::SubtaskInvalid)?;
        let receipt = self.start_with_cleanup(child_id, None, operation, cleanup)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        // The parent can stop or finish between the initial validation and
        // child admission. The receipt still owns the started child; cancel it
        // instead of reporting an admission failure after it may have run.
        let parent_stopped = state.tasks.get(&parent_id).is_none_or(|task| {
            task.run_id != parent_run_id || task.finished() || task.cancellation.is_cancelled()
        });
        if !parent_stopped {
            state
                .children
                .entry(parent_id)
                .or_default()
                .insert(child_id);
        } else {
            receipt.0.cancellation.cancel();
            Self::cancel_descendants(&state, child_id);
        }
        Ok(receipt)
    }

    /// Keep admission reserved through reconciliation, including after a panic.
    pub(crate) fn start_with_cleanup<F, Fut, C, Cleanup>(
        &self,
        conversation_id: ConversationId,
        turn_id: Option<Uuid>,
        operation: F,
        cleanup: C,
    ) -> Result<TaskReceipt, TaskRegistryError>
    where
        F: FnOnce(AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
        C: FnOnce(TaskCompletion) -> Cleanup + Send + 'static,
        Cleanup: Future<Output = TaskCompletion> + Send + 'static,
    {
        self.start_with_cleanup_claim(
            conversation_id,
            turn_id,
            Arc::new(AtomicBool::new(false)),
            operation,
            cleanup,
        )
    }

    pub(super) fn start_with_cleanup_claim<F, Fut, C, Cleanup>(
        &self,
        conversation_id: ConversationId,
        turn_id: Option<Uuid>,
        turn_owned: Arc<AtomicBool>,
        operation: F,
        cleanup: C,
    ) -> Result<TaskReceipt, TaskRegistryError>
    where
        F: FnOnce(AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
        C: FnOnce(TaskCompletion) -> Cleanup + Send + 'static,
        Cleanup: Future<Output = TaskCompletion> + Send + 'static,
    {
        self.start_with_cleanup_claim_id(
            conversation_id,
            turn_id,
            turn_owned,
            Uuid::new_v4(),
            operation,
            |result| async move {
                TaskObservation {
                    completion: cleanup(result).await,
                    diagnostic: None,
                }
            },
        )
    }

    pub(super) fn start_with_cleanup_claim_id<F, Fut, C, Cleanup>(
        &self,
        conversation_id: ConversationId,
        turn_id: Option<Uuid>,
        turn_owned: Arc<AtomicBool>,
        run_id: Uuid,
        operation: F,
        cleanup: C,
    ) -> Result<TaskReceipt, TaskRegistryError>
    where
        F: FnOnce(AgentCancellation) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
        C: FnOnce(TaskCompletion) -> Cleanup + Send + 'static,
        Cleanup: Future<Output = TaskObservation> + Send + 'static,
    {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| TaskRegistryError::Unavailable)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        if state.closing {
            return Err(TaskRegistryError::Closing);
        }
        Self::retain_finished(&mut state);
        Self::prune_lineage(&mut state);
        if state.tasks.contains_key(&conversation_id)
            || state.reservations.contains(&conversation_id)
        {
            return Err(TaskRegistryError::Active);
        }
        if state.tasks.len()
            + state
                .reservations
                .len()
                .saturating_sub(state.batch_reservations.len())
            >= MAX_TASKS
        {
            return Err(TaskRegistryError::Capacity);
        }
        let cancellation = AgentCancellation::default();
        let signal = cancellation.clone();
        let handle = runtime.spawn(async move {
            // The supervisor owns and awaits this child. It is never aborted by
            // stop/shutdown; even a panicking operation reaches reconciliation.
            let (result, diagnostic) = match tokio::spawn(async move { operation(signal).await })
                .await
            {
                Ok(Ok(())) => (TaskCompletion::Completed, None),
                Ok(Err(RunError::Model(ModelError::Canceled))) => (TaskCompletion::Canceled, None),
                Ok(Err(
                    RunError::EventSinkDiagnostic(detail)
                    | RunError::ToolOutcomeUnknownDiagnostic(detail),
                )) => (TaskCompletion::RecoveryRequired, Some(*detail)),
                Ok(Err(RunError::Model(error @ ModelError::PersistenceDiagnostic(_)))) => {
                    let diagnostic = match RunError::from(error) {
                        RunError::EventSinkDiagnostic(detail) => Some(*detail),
                        _ => None,
                    };
                    (TaskCompletion::RecoveryRequired, diagnostic)
                }
                Ok(Err(error)) if error.is_event_sink() => (TaskCompletion::RecoveryRequired, None),
                Ok(Err(RunError::ToolOutcomeUnknown)) | Err(_) => {
                    (TaskCompletion::RecoveryRequired, None)
                }
                Ok(Err(_)) => (TaskCompletion::Failed, None),
            };
            // A cleanup panic must not discard the original safe diagnosis.
            let cleanup = tokio::spawn(async move { cleanup(result).await })
                .await
                .unwrap_or(TaskObservation {
                    completion: TaskCompletion::RecoveryRequired,
                    diagnostic: None,
                });
            TaskObservation {
                completion: cleanup.completion,
                diagnostic: diagnostic.or(cleanup.diagnostic),
            }
        });
        let entry = Arc::new(TaskEntry {
            run_id,
            turn_id,
            turn_owned,
            cancellation,
            completion: handle.abort_handle(),
            join: AsyncMutex::new(TaskJoin {
                handle: Some(handle),
                result: None,
            }),
        });
        state.tasks.insert(conversation_id, entry.clone());
        Ok(TaskReceipt(entry))
    }

    /// A stale stop request cannot cancel a later task in the same conversation.
    pub fn cancel(
        &self,
        conversation_id: ConversationId,
        run_id: Uuid,
    ) -> Result<bool, TaskRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        let matching = state.tasks.get(&conversation_id).cloned();
        if let Some(task) = matching
            && task.run_id == run_id
            && !task.finished()
        {
            task.cancellation.cancel();
            Self::cancel_descendants(&state, conversation_id);
            return Ok(true);
        }
        Ok(false)
    }

    /// Matches durable identity under the registry lock. This only requests
    /// cancellation; the owner still completes IO/reconciliation and its terminal.
    pub fn cancel_turn(
        &self,
        conversation_id: ConversationId,
        turn_id: Uuid,
    ) -> Result<bool, TaskRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TaskRegistryError::Unavailable)?;
        if let Some(task) = state.tasks.get(&conversation_id)
            && task.turn_id == Some(turn_id)
            && !task.finished()
        {
            task.cancellation.cancel();
            Self::cancel_descendants(&state, conversation_id);
            return Ok(true);
        }
        Ok(false)
    }

    /// The caller must first fence all admission to this exact set and await
    /// its in-flight preparation. This waits only these registered owners;
    /// it does not promise a drain of unknown or concurrently linked children.
    #[cfg(test)]
    pub(crate) async fn cancel_and_wait_scope(
        &self,
        conversations: &[ConversationId],
    ) -> Result<Vec<(ConversationId, TaskCompletion)>, TaskRegistryError> {
        let tasks = {
            let state = self
                .state
                .lock()
                .map_err(|_| TaskRegistryError::Unavailable)?;
            conversations
                .iter()
                .filter_map(|id| {
                    state.tasks.get(id).map(|task| {
                        task.cancellation.cancel();
                        (*id, task.clone())
                    })
                })
                .collect::<Vec<_>>()
        };
        let mut results = Vec::with_capacity(tasks.len());
        for (id, task) in tasks {
            results.push((id, task.wait().await.completion));
        }
        Ok(results)
    }

    /// Stops admission before signaling every task. Can be resumed if the waiter
    /// is canceled: entries and JoinHandles remain owned until completion.
    pub async fn shutdown(
        &self,
    ) -> Result<Vec<(ConversationId, TaskCompletion)>, TaskRegistryError> {
        let tasks = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| TaskRegistryError::Unavailable)?;
            state.closing = true;
            state
                .tasks
                .iter()
                .map(|(id, task)| {
                    task.cancellation.cancel();
                    (*id, task.clone())
                })
                .collect::<Vec<_>>()
        };
        let mut results = Vec::with_capacity(tasks.len());
        for (id, task) in tasks {
            results.push((id, task.wait().await.completion));
        }
        self.wait_for_reservations().await?;
        Ok(results)
    }
}
impl Drop for AgentTaskRegistry {
    fn drop(&mut self) {
        // Best effort only. Application exit must await shutdown while the runtime
        // is alive; Drop cannot promise async flush or replace crash recovery.
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner());
        for task in state.tasks.values() {
            task.cancellation.cancel();
        }
    }
}

#[cfg(test)]
#[path = "tasks/tests.rs"]
mod tests;
