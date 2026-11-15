//! Application-scoped session registry. UI lifetimes do not own these sessions.

#[path = "service/compaction_hooks.rs"]
mod compaction_hooks;
#[path = "service/context_usage.rs"]
mod context_usage;
#[path = "service/context_window.rs"]
mod context_window;
#[path = "service/deletion.rs"]
mod deletion;
#[path = "service/history_snapshot.rs"]
pub(crate) mod history_snapshot;
#[path = "service/legacy_import.rs"]
mod legacy_import;
#[path = "service/legacy_recovery.rs"]
mod legacy_recovery;
#[path = "service/notifications.rs"]
mod notifications;
#[path = "service/payloads.rs"]
mod payloads;
#[path = "service/rollback.rs"]
mod rollback;
#[path = "service/run.rs"]
mod run;
#[path = "service/token_budget.rs"]
mod token_budget;
pub use rollback::ConversationRollbackRequest;
#[path = "service/shutdown.rs"]
mod shutdown;
#[path = "service/steering.rs"]
mod steering;
#[path = "service/submission.rs"]
mod submission;
#[cfg(unix)]
#[path = "service/workspace_discard.rs"]
mod workspace_discard;
pub use steering::SessionTurnInput;
#[path = "service/accepted_history.rs"]
mod accepted_history;
#[path = "service/updates.rs"]
mod updates;
pub(crate) use accepted_history::AcceptedHistoryUpdate;
pub use run::OwnedTurn;
pub use submission::{SubmissionAdmission, SubmissionRunError};

use research_agent_core::{
    conversation::{
        ConversationId, ConversationStore, HistoryPage, HistoryQuery, StoreError, WriterId,
        WriterLease,
    },
    protocol::{AgentEvent, ConversationProjection},
    session::{CommittedBatch, DurableConversation, SessionError},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use uuid::Uuid;

type Slot = Arc<AsyncMutex<Option<DurableConversation>>>;
const MAX_OPEN_SESSIONS: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum AgentServiceError {
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Storage(#[from] StoreError),
    #[error("agent session registry unavailable")]
    Unavailable,
    #[error("too many open agent sessions")]
    Capacity,
    #[error("agent session is not open")]
    NotOpen,
    #[error("agent session is still active")]
    Active,
    #[error("conversation rollback history conflict")]
    RollbackConflict,
    #[error("legacy reading history import is incomplete: {0}")]
    LegacyMigrationIncomplete(Uuid),
}

/// A wakeup carrying only committed identity and sequence. Consumers call read_since;
/// broadcast lag or no listeners never loses durable events or duplicates execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitNotice {
    pub conversation_id: ConversationId,
    pub watermark: u64,
}

pub struct AgentSessionService {
    store: Arc<dyn ConversationStore<AgentEvent>>,
    compaction_db: Option<crate::data_interface::SharedDatabase>,
    sessions: Mutex<HashMap<ConversationId, Slot>>,
    batch_fence_slots: Mutex<HashSet<ConversationId>>,
    deleting: Mutex<HashSet<ConversationId>>,
    notices: broadcast::Sender<CommitNotice>,
    accepted_history: Mutex<accepted_history::AcceptedHistory>,
    history_snapshots: Arc<Mutex<history_snapshot::SnapshotCache>>,
    activity_changed: Option<Arc<dyn Fn() + Send + Sync>>,
    subtasks_changed: Option<Arc<dyn Fn() + Send + Sync>>,
    tasks: super::tasks::AgentTaskRegistry,
    #[cfg(unix)]
    processes: Arc<super::process::ProcessRegistry>,
    #[cfg(unix)]
    mcp_runtime: Arc<tokio::sync::RwLock<Option<Arc<super::mcp_runtime::McpRuntime>>>>,
    update_waiters: tokio::sync::Semaphore,
}

impl AgentSessionService {
    #[cfg(target_os = "macos")]
    pub(crate) fn has_logged_data(&self) -> bool {
        self.compaction_db.as_ref().is_some_and(|db| db.logged())
    }
    pub(crate) fn owns_data(&self, database: &crate::data_interface::SharedDatabase) -> bool {
        self.compaction_db
            .as_ref()
            .is_some_and(|owned| Arc::ptr_eq(owned, database))
    }
    pub(crate) fn from_app_store(store: &crate::storage::AppStore) -> Result<Self, StoreError> {
        {
            let mut db = store.connection().map_err(|_| StoreError::Unavailable)?;
            super::repository::input_queue::pause_on_startup(&mut db)
                .map_err(|_| StoreError::Unavailable)?;
            super::repository::proposal_applications::recover_on_startup(&mut db)
                .map_err(|_| StoreError::Unavailable)?;
            super::repository::proposal_undos::recover_on_startup(&mut db)
                .map_err(|_| StoreError::Unavailable)?;
            // Legacy domain publication needs its selected owner. Keep this
            // chain recoverable while ordinary Agent startup recovery proceeds.
            if store.generation_policy_source().is_some() {
                crate::ai::reconcile_legacy_runs_on_startup(&mut db)?;
            }
        }
        let repository = super::repository::SqliteConversationStore::new(
            store.shared_agent_connection(),
            "新对话".into(),
        )?;
        let mut service = Self::new_with_compaction_db(
            Arc::new(repository),
            Some(store.shared_agent_connection()),
        );
        // Only already committed tombstones authorize this cleanup. An
        // interrupted batch itself is never resumed on startup.
        let catalog = super::repository::SqliteConversationCatalog::from_app_store(store);
        let _cleanup = tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = catalog.collect_deleted_workspaces() {
                eprintln!("agent deleted-workspace cleanup deferred: {error}");
            }
        });
        service.install_app_notifications(store);
        Ok(service)
    }
    #[cfg(test)]
    pub fn new(store: Arc<dyn ConversationStore<AgentEvent>>) -> Self {
        Self::new_with_compaction_db(store, None::<crate::data_interface::SharedDatabase>)
    }

    fn new_with_compaction_db<D: crate::data_interface::IntoDatabase>(
        store: Arc<dyn ConversationStore<AgentEvent>>,
        compaction_db: Option<D>,
    ) -> Self {
        let (notices, _) = broadcast::channel(128);
        Self {
            store,
            compaction_db: compaction_db.map(crate::data_interface::IntoDatabase::into_database),
            sessions: Mutex::new(HashMap::new()),
            batch_fence_slots: Mutex::new(HashSet::new()),
            deleting: Mutex::new(HashSet::new()),
            notices,
            accepted_history: Mutex::new(accepted_history::AcceptedHistory::default()),
            history_snapshots: Arc::default(),
            activity_changed: None,
            subtasks_changed: None,
            tasks: super::tasks::AgentTaskRegistry::default(),
            #[cfg(unix)]
            processes: Arc::new(super::process::ProcessRegistry::default()),
            #[cfg(unix)]
            mcp_runtime: Arc::new(tokio::sync::RwLock::new(None)),
            update_waiters: tokio::sync::Semaphore::new(128),
        }
    }

    pub(crate) async fn active_context_checkpoint(
        &self,
        conversation_id: Uuid,
    ) -> Result<
        Option<research_agent_core::context::checkpoint::ContextCheckpoint>,
        research_agent_core::context::checkpoint_store::CheckpointStoreError,
    > {
        use research_agent_core::context::checkpoint_store::ContextCheckpointStore;
        let Some(connection) = self.compaction_db.clone() else {
            return Ok(None);
        };
        super::repository::context_window_store::SqliteContextCheckpointStore::new(connection)
            .read_active(conversation_id)
            .await
    }

    #[cfg(unix)]
    pub fn processes(&self) -> &Arc<super::process::ProcessRegistry> {
        &self.processes
    }

    pub fn tasks(&self) -> &super::tasks::AgentTaskRegistry {
        &self.tasks
    }

    pub(crate) fn task_settlement_database(&self) -> Option<crate::data_interface::SharedDatabase> {
        self.compaction_db
            .clone()
            .filter(|database| database.logged())
    }

    #[cfg(unix)]
    pub async fn install_mcp_runtime(&self, runtime: Arc<super::mcp_runtime::McpRuntime>) {
        *self.mcp_runtime.write().await = Some(runtime);
    }

    #[cfg(unix)]
    pub async fn clear_mcp_runtime(&self) {
        self.mcp_runtime.write().await.take();
    }

    #[cfg(unix)]
    pub async fn mcp_runtime(&self) -> Option<Arc<super::mcp_runtime::McpRuntime>> {
        self.mcp_runtime.read().await.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CommitNotice> {
        self.notices.subscribe()
    }

    fn slot(&self, id: ConversationId, create: bool) -> Result<Slot, AgentServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| AgentServiceError::Unavailable)?;
        if let Some(slot) = sessions.get(&id) {
            return Ok(slot.clone());
        }
        if !create {
            return Err(AgentServiceError::NotOpen);
        }
        // Failed/canceled initialization may leave an empty slot. Reclaim only when
        // no contender holds it; otherwise two registries could open the same ID.
        sessions.retain(|_, slot| {
            Arc::strong_count(slot) != 1
                || match slot.try_lock() {
                    Ok(value) => value.is_some(),
                    Err(_) => true,
                }
        });
        let fences = self
            .batch_fence_slots
            .lock()
            .map_err(|_| AgentServiceError::Unavailable)?;
        if sessions.keys().filter(|id| !fences.contains(id)).count() >= MAX_OPEN_SESSIONS {
            return Err(AgentServiceError::Capacity);
        }
        let slot = Arc::new(AsyncMutex::new(None));
        sessions.insert(id, slot.clone());
        Ok(slot)
    }

    pub async fn create(
        &self,
        id: ConversationId,
    ) -> Result<ConversationProjection, AgentServiceError> {
        let slot = self.slot(id, true)?;
        let mut owner = slot.lock().await;
        if owner.is_some() {
            return Err(StoreError::AlreadyExists.into());
        }
        let session = DurableConversation::create(self.store.clone(), fresh_lease(id)).await?;
        let projection = session.projection().clone();
        *owner = Some(session);
        Ok(projection)
    }

    /// Reuses a live session across hosts. A disk-only session is recovered once,
    /// with an interrupted terminal record for any run left active by a prior process.
    pub async fn open(
        &self,
        id: ConversationId,
    ) -> Result<ConversationProjection, AgentServiceError> {
        let slot = self.slot(id, true)?;
        // A live turn keeps the durable writer lock while model/tool work is in
        // progress. Opening a history view must not wait for that work to finish:
        // the renderer can rebuild its projection from committed events while the
        // owner continues to serialize writes. This is the same read-only path as
        // `read_since`; it never installs a second writer or permits a commit.
        let deletion_in_progress = self
            .deleting
            .lock()
            .map_err(|_| AgentServiceError::Unavailable)?
            .contains(&id);
        let mut owner = if deletion_in_progress {
            // A deletion intentionally holds the slot while its durable catalog
            // transaction settles. Do not use the live-writer read projection
            // shortcut here: it could expose a conversation that is already
            // being deleted and let a queued open resurrect its view.
            slot.lock().await
        } else {
            match slot.try_lock() {
                Ok(owner) => owner,
                Err(_) => return self.read_projection(id).await,
            }
        };
        if let Some(session) = owner.as_ref() {
            if session.needs_recovery() {
                return Err(SessionError::RecoveryRequired.into());
            }
            return Ok(session.projection().clone());
        }
        let mut session = DurableConversation::resume(self.store.clone(), fresh_lease(id)).await?;
        self.check_legacy_recovery(id, &session).await?;
        if let Some(receipt) = session
            .interrupt_recovered_turn(Uuid::new_v4(), now_ms()?)
            .await?
        {
            self.notify(id, &receipt);
        }
        let projection = session.projection().clone();
        *owner = Some(session);
        Ok(projection)
    }

    pub(crate) async fn read_projection(
        &self,
        id: ConversationId,
    ) -> Result<ConversationProjection, AgentServiceError> {
        let mut projection = ConversationProjection::new(id);
        let mut after_seq = 0;
        // History pages are individually consistent SQLite reads. A live writer can
        // append between pages, so the first page defines the snapshot we are
        // rebuilding and later pages must only prove that this prefix still exists.
        // Requiring every page to report the same durable count made a perfectly
        // healthy active conversation look corrupt when its turn kept running.
        let mut snapshot_count: Option<u64> = None;
        loop {
            let remaining = snapshot_count
                .map(|count| count.saturating_sub(after_seq))
                .unwrap_or(500);
            if remaining == 0 {
                break;
            }
            let page = self
                .store
                .history(HistoryQuery::new(id, after_seq, remaining.min(500) as u16)?)
                .await?;
            let count = *snapshot_count.get_or_insert(page.durable_count);
            if page.durable_count < count {
                return Err(StoreError::CorruptHistory.into());
            }
            let next_offset = page.next_offset;
            let previous = projection.seq();
            for event in page.items {
                projection.apply(&event).map_err(SessionError::from)?;
            }
            if projection.seq() > count || projection.seq() == previous && projection.seq() < count
            {
                return Err(StoreError::CorruptHistory.into());
            }
            after_seq = projection.seq();
            if after_seq == count {
                break;
            }
            if next_offset != Some(after_seq) {
                return Err(StoreError::CorruptHistory.into());
            }
        }
        if projection.seq() != snapshot_count.unwrap_or(0) {
            return Err(StoreError::CorruptHistory.into());
        }
        Ok(projection)
    }

    /// Idempotent creation identity supplied by the client. Never resets an
    /// existing conversation or replaces a healthy active writer on a retry.
    pub async fn create_or_open(
        &self,
        id: ConversationId,
    ) -> Result<ConversationProjection, AgentServiceError> {
        match self.create(id).await {
            Ok(projection) => Ok(projection),
            Err(AgentServiceError::Storage(StoreError::AlreadyExists))
            | Err(AgentServiceError::Session(SessionError::Storage(StoreError::AlreadyExists))) => {
                self.open(id).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn commit(
        &self,
        id: ConversationId,
        events: Vec<AgentEvent>,
    ) -> Result<CommittedBatch, AgentServiceError> {
        let slot = self.slot(id, false)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(AgentServiceError::NotOpen)?;
        let receipt = session.commit(events).await?;
        self.notify(id, &receipt);
        Ok(receipt)
    }

    /// Assign sequence and commit under the same session lock; a runner must not
    /// derive a new seq from a snapshot that another producer could advance.
    pub async fn commit_payload(
        &self,
        id: ConversationId,
        request_id: Uuid,
        payload: research_agent_core::protocol::EventPayload,
    ) -> Result<(), AgentServiceError> {
        self.commit_payloads(id, request_id, vec![payload]).await?;
        Ok(())
    }

    /// Atomically acknowledge related payloads (for example turn start and input).
    /// Sequence allocation, full validation and flush share one session lock.
    pub async fn commit_payloads(
        &self,
        id: ConversationId,
        request_id: Uuid,
        payloads: Vec<research_agent_core::protocol::EventPayload>,
    ) -> Result<CommittedBatch, AgentServiceError> {
        if payloads.is_empty() || payloads.len() > 500 {
            return Err(SessionError::InvalidBatch.into());
        }
        let slot = self.slot(id, false)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(AgentServiceError::NotOpen)?;
        self.commit_payloads_owned(id, request_id, payloads, session)
            .await
    }

    /// Explicitly reconcile a failed/canceled commit. A healthy live session cannot
    /// be replaced through this path, so another host cannot reset a running task.
    pub async fn recover(
        &self,
        id: ConversationId,
    ) -> Result<ConversationProjection, AgentServiceError> {
        let slot = self.slot(id, false)?;
        let mut owner = slot.lock().await;
        if owner.as_ref().is_some_and(|value| !value.needs_recovery()) {
            return Err(AgentServiceError::Active);
        }
        owner.take();
        let mut session = DurableConversation::resume(self.store.clone(), fresh_lease(id)).await?;
        self.check_legacy_recovery(id, &session).await?;
        if let Some(receipt) = session
            .interrupt_recovered_turn(Uuid::new_v4(), now_ms()?)
            .await?
        {
            self.notify(id, &receipt);
        }
        let projection = session.projection().clone();
        *owner = Some(session);
        Ok(projection)
    }

    /// after_seq is the last event received (0 for none), also the next zero-based
    /// journal ordinal. This reads durable history without opening a live writer.
    pub async fn read_since(
        &self,
        id: ConversationId,
        after_seq: u64,
        limit: u16,
    ) -> Result<HistoryPage<AgentEvent>, AgentServiceError> {
        Ok(self
            .store
            .history(HistoryQuery::new(id, after_seq, limit)?)
            .await?)
    }

    /// Explicit resource release, never called just because a chat panel is hidden.
    pub async fn release_idle(&self, id: ConversationId) -> Result<(), AgentServiceError> {
        let slot = self.slot(id, false)?;
        let mut owner = slot.lock().await;
        if owner.as_ref().is_some_and(|session| {
            session.needs_recovery() || session.projection().active_turn().is_some()
        }) {
            return Err(AgentServiceError::Active);
        }
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| AgentServiceError::Unavailable)?;
        if Arc::strong_count(&slot) != 2 {
            return Err(AgentServiceError::Active);
        }
        owner.take();
        sessions.remove(&id);
        Ok(())
    }

    fn notify(&self, id: ConversationId, receipt: &CommittedBatch) {
        self.accepted_history
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(id, receipt);
        // No listeners is normal when all panels are hidden. Durable history remains.
        let _ = self.notices.send(CommitNotice {
            conversation_id: id,
            watermark: receipt.watermark,
        });
        self.notify_lifecycle(receipt);
    }
}

fn fresh_lease(conversation_id: ConversationId) -> WriterLease {
    WriterLease {
        conversation_id,
        writer_id: WriterId::from_uuid(Uuid::new_v4()),
    }
}
fn now_ms() -> Result<u64, AgentServiceError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .ok_or(AgentServiceError::Unavailable)
}

#[cfg(test)]
#[path = "service/tests.rs"]
mod tests;
