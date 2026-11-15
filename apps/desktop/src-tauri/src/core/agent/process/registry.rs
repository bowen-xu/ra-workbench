//! App-owned process handles scoped to the authoritative workspace identity.
//! Native admission must authorize/sandbox the launcher; scope is not a grant.
use super::{
    ProcessError, ProcessPoll, ProcessSession, TerminalSize, session::ProcessCancellation,
};
use research_agent_core::workspace::{WorkspaceIdentity, WorkspaceManifest};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ProcessRegistryError {
    #[error("invalid process scope")]
    Invalid,
    #[error("process is absent or belongs to another scope")]
    NotFound,
    #[error("process registry is closing")]
    Closing,
    #[error("process registry is full")]
    Capacity,
    #[error("process still owns pending work")]
    Busy,
    #[error("process registry unavailable")]
    Unavailable,
    #[error(transparent)]
    Process(#[from] ProcessError),
}
struct Entry {
    scope: WorkspaceIdentity,
    session: AsyncMutex<ProcessSession>,
    cancel: ProcessCancellation,
}
#[derive(Default)]
struct State {
    closing: bool,
    entries: HashMap<Uuid, Arc<Entry>>,
    active_turns: HashSet<(Uuid, Uuid)>,
}
#[derive(Default)]
pub struct ProcessRegistry {
    state: Mutex<State>,
}
impl ProcessRegistry {
    /// A delivered workspace must not strand an unacknowledged process result.
    /// Call while holding the workspace owner so a launch cannot race the check.
    pub(crate) fn ensure_scope_empty(
        &self,
        scope: &WorkspaceIdentity,
    ) -> Result<(), ProcessRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        if state.entries.values().any(|entry| &entry.scope == scope) {
            Err(ProcessRegistryError::Busy)
        } else {
            Ok(())
        }
    }
    /// Called only inside the application-owned turn operation, after task admission.
    pub(crate) fn begin_turn(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<(), ProcessRegistryError> {
        if conversation.is_nil() || turn.is_nil() {
            return Err(ProcessRegistryError::Invalid);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        if state.closing {
            return Err(ProcessRegistryError::Closing);
        }
        if state.active_turns.len() >= 128 {
            return Err(ProcessRegistryError::Capacity);
        }
        if !state.active_turns.insert((conversation, turn)) {
            return Err(ProcessRegistryError::Busy);
        }
        Ok(())
    }
    pub(crate) fn stop_turn(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<(), ProcessRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        state.active_turns.remove(&(conversation, turn));
        for entry in state.entries.values().filter(|entry| {
            entry.scope.conversation_id == conversation && entry.scope.turn_id == turn
        }) {
            entry.cancel.cancel();
        }
        Ok(())
    }
    /// Close admission before signaling cancellation. Retain results for explicit
    /// observation/forget; failure keeps unresolved process ownership intact.
    pub(crate) async fn finish_turn(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<(), ProcessRegistryError> {
        let entries: Vec<_> = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ProcessRegistryError::Unavailable)?;
            state.active_turns.remove(&(conversation, turn));
            state
                .entries
                .values()
                .filter(|entry| {
                    entry.scope.conversation_id == conversation && entry.scope.turn_id == turn
                })
                .map(|entry| {
                    entry.cancel.cancel();
                    entry.clone()
                })
                .collect()
        };
        let mut failed = false;
        for entry in entries {
            let mut session = entry.session.lock().await;
            if session.finish().await.is_err() || !session.safely_finished() {
                failed = true;
            }
        }
        if failed {
            Err(ProcessRegistryError::Busy)
        } else {
            Ok(())
        }
    }

    /// Blocking-worker only: admission is serialized with synchronous spawning.
    /// Capacity/scope rejection happens before the launcher can create effects.
    pub fn start(
        &self,
        scope: WorkspaceIdentity,
        launch: impl FnOnce() -> Result<ProcessSession, ProcessError>,
    ) -> Result<Uuid, ProcessRegistryError> {
        WorkspaceManifest {
            schema_version: 1,
            identity: scope.clone(),
            files: vec![],
        }
        .validate()
        .map_err(|_| ProcessRegistryError::Invalid)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        if state.closing {
            return Err(ProcessRegistryError::Closing);
        }
        if state.entries.len() >= 16 {
            return Err(ProcessRegistryError::Capacity);
        }
        if state
            .entries
            .values()
            .any(|entry| entry.scope.lease_id == scope.lease_id)
        {
            return Err(ProcessRegistryError::Busy);
        }
        if !state
            .active_turns
            .contains(&(scope.conversation_id, scope.turn_id))
        {
            return Err(ProcessRegistryError::Closing);
        }
        let session = launch()?;
        let id = Uuid::new_v4();
        state.entries.insert(
            id,
            Arc::new(Entry {
                scope,
                cancel: session.cancellation(),
                session: AsyncMutex::new(session),
            }),
        );
        Ok(id)
    }
    fn entry(
        &self,
        scope: &WorkspaceIdentity,
        id: Uuid,
    ) -> Result<Arc<Entry>, ProcessRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        state
            .entries
            .get(&id)
            .filter(|entry| &entry.scope == scope)
            .cloned()
            .ok_or(ProcessRegistryError::NotFound)
    }
    pub async fn poll(
        &self,
        scope: &WorkspaceIdentity,
        id: Uuid,
        interval: Duration,
    ) -> Result<ProcessPoll, ProcessRegistryError> {
        Ok(self
            .entry(scope, id)?
            .session
            .lock()
            .await
            .poll(interval)
            .await?)
    }
    pub async fn write(
        &self,
        scope: &WorkspaceIdentity,
        id: Uuid,
        bytes: &[u8],
    ) -> Result<usize, ProcessRegistryError> {
        if bytes.len() > 64 * 1024 {
            return Err(ProcessRegistryError::Invalid);
        }
        Ok(self
            .entry(scope, id)?
            .session
            .lock()
            .await
            .write(bytes)
            .await?)
    }
    pub async fn close_input(
        &self,
        scope: &WorkspaceIdentity,
        id: Uuid,
    ) -> Result<(), ProcessRegistryError> {
        self.entry(scope, id)?.session.lock().await.close_input();
        Ok(())
    }
    pub async fn resize(
        &self,
        scope: &WorkspaceIdentity,
        id: Uuid,
        size: TerminalSize,
    ) -> Result<(), ProcessRegistryError> {
        Ok(self
            .entry(scope, id)?
            .session
            .lock()
            .await
            .resize(size)
            .await?)
    }
    pub async fn interrupt(
        &self,
        scope: &WorkspaceIdentity,
        id: Uuid,
    ) -> Result<(), ProcessRegistryError> {
        Ok(self
            .entry(scope, id)?
            .session
            .lock()
            .await
            .interrupt()
            .await?)
    }
    pub fn cancel(&self, scope: &WorkspaceIdentity, id: Uuid) -> Result<(), ProcessRegistryError> {
        self.entry(scope, id)?.cancel.cancel();
        Ok(())
    }
    pub fn cancel_turn(&self, conversation: Uuid, turn: Uuid) -> Result<(), ProcessRegistryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        for entry in state.entries.values().filter(|entry| {
            entry.scope.conversation_id == conversation && entry.scope.turn_id == turn
        }) {
            entry.cancel.cancel();
        }
        Ok(())
    }
    /// Completed output remains counted until explicitly forgotten after observation.
    pub fn forget(&self, scope: &WorkspaceIdentity, id: Uuid) -> Result<(), ProcessRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        let entry = state
            .entries
            .get(&id)
            .filter(|entry| &entry.scope == scope)
            .ok_or(ProcessRegistryError::NotFound)?;
        if !entry
            .session
            .try_lock()
            .is_ok_and(|session| session.safely_finished())
        {
            return Err(ProcessRegistryError::Busy);
        }
        state.entries.remove(&id);
        Ok(())
    }
    pub fn begin_shutdown(&self) -> Result<(), ProcessRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?;
        state.closing = true;
        state.active_turns.clear();
        for entry in state.entries.values() {
            entry.cancel.cancel();
        }
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<(), ProcessRegistryError> {
        self.begin_shutdown()?;
        let entries: Vec<_> = self
            .state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?
            .entries
            .values()
            .cloned()
            .collect();
        let mut failed = false;
        for entry in entries {
            let mut session = entry.session.lock().await;
            if session.finish().await.is_err() || !session.safely_finished() {
                failed = true;
            }
        }
        if failed {
            return Err(ProcessRegistryError::Busy);
        }
        self.state
            .lock()
            .map_err(|_| ProcessRegistryError::Unavailable)?
            .entries
            .clear();
        Ok(())
    }
}
impl Drop for ProcessRegistry {
    fn drop(&mut self) {
        let _ = self.begin_shutdown();
    }
}
#[cfg(test)]
#[path = "registry/tests.rs"]
mod tests;
