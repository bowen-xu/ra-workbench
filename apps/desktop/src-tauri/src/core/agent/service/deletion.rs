//! Deletion owns admission and the session until durable deletion has settled.
use super::*;
use crate::agent::tasks::TaskRegistryError;
use research_agent_core::conversation::{
    CatalogError, ConversationDeletionError as Error, ConversationDeletionReceipt,
    ConversationDeletionRequest, ConversationDeletionStore,
};

struct DeletionMarker {
    service: Arc<AgentSessionService>,
    conversation_id: ConversationId,
}

struct CancelBatchOnDrop(Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelBatchOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for DeletionMarker {
    fn drop(&mut self) {
        if let Ok(mut deleting) = self.service.deleting.lock() {
            deleting.remove(&self.conversation_id);
        }
        if let Ok(mut fences) = self.service.batch_fence_slots.lock() {
            fences.remove(&self.conversation_id);
        }
    }
}

impl AgentSessionService {
    pub(crate) async fn delete_conversations_batch(
        self: &Arc<Self>,
        catalog: crate::agent::repository::SqliteConversationCatalog,
        requests: Vec<ConversationDeletionRequest>,
    ) -> Result<Vec<Result<(ConversationDeletionReceipt, bool), Error>>, Error> {
        let mut ids = std::collections::HashSet::new();
        let mut operations = std::collections::HashSet::new();
        if requests.is_empty()
            || requests.iter().any(|request| {
                request.validate().is_err()
                    || !ids.insert(request.conversation_id)
                    || !operations.insert(request.request_id)
            })
        {
            return Err(CatalogError::InvalidInput.into());
        }
        let service = self.clone();
        let canceled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _cancel_on_drop = CancelBatchOnDrop(canceled.clone());
        tokio::spawn(async move {
            let mut results: Vec<Option<Result<(ConversationDeletionReceipt, bool), Error>>> =
                (0..requests.len()).map(|_| None).collect();
            let mut markers = Vec::new();
            let mut reservations = Vec::new();
            let mut owners = Vec::new();
            let mut prepared = Vec::new();
            let mut included = Vec::new();
            for (index, request) in requests.iter().enumerate() {
                if canceled.load(std::sync::atomic::Ordering::Acquire)
                    || service.tasks.is_closing().unwrap_or(true)
                {
                    return Err(CatalogError::Unavailable.into());
                }
                let id = request.conversation_id;
                let inserted = service
                    .deleting
                    .lock()
                    .map_err(|_| CatalogError::Unavailable)?
                    .insert(id);
                if !inserted {
                    results[index] = Some(Err(CatalogError::Busy.into()));
                    continue;
                }
                let marker = DeletionMarker {
                    service: service.clone(),
                    conversation_id: id,
                };
                let reservation = match service.tasks.reserve_idle_for_batch(id) {
                    Ok(value) => value,
                    Err(TaskRegistryError::Active | TaskRegistryError::Capacity) => {
                        results[index] = Some(Err(CatalogError::Busy.into()));
                        continue;
                    }
                    Err(_) => {
                        results[index] = Some(Err(CatalogError::Unavailable.into()));
                        continue;
                    }
                };
                // Batch-held empty slots fence concurrent open/create but are not
                // live sessions and therefore do not consume the open-session cap.
                let slot = {
                    let mut sessions = service
                        .sessions
                        .lock()
                        .map_err(|_| CatalogError::Unavailable)?;
                    if let Some(slot) = sessions.get(&id) {
                        slot.clone()
                    } else {
                        let slot = Arc::new(AsyncMutex::new(None));
                        service
                            .batch_fence_slots
                            .lock()
                            .map_err(|_| CatalogError::Unavailable)?
                            .insert(id);
                        sessions.insert(id, slot.clone());
                        slot
                    }
                };
                let mut owner = slot.lock_owned().await;
                if canceled.load(std::sync::atomic::Ordering::Acquire)
                    || service.tasks.is_closing().unwrap_or(true)
                {
                    return Err(CatalogError::Unavailable.into());
                }
                if owner.as_ref().is_some_and(|session| {
                    session.needs_recovery() || session.projection().active_turn().is_some()
                }) {
                    results[index] = Some(Err(CatalogError::Busy.into()));
                    continue;
                }
                owner.take();
                match catalog.prepare_batch_deletion(request.clone()).await {
                    Ok(value) => {
                        prepared.push(value);
                        included.push(index);
                        markers.push(marker);
                        reservations.push(reservation);
                        owners.push(owner);
                    }
                    Err(error) => results[index] = Some(Err(error)),
                }
            }
            if !prepared.is_empty() {
                if canceled.load(std::sync::atomic::Ordering::Acquire)
                    || service.tasks.is_closing().unwrap_or(true)
                {
                    return Err(CatalogError::Unavailable.into());
                }
                catalog.publish_deletion_batch(prepared).await?;
                // Cleanup is restricted to committed tombstones; a failure keeps
                // the durable queue and is reported for the affected operation.
                let collector = catalog.clone();
                let pending = tokio::task::spawn_blocking(move || {
                    let _ = collector.collect_deleted_workspaces();
                    collector.pending_deleted_workspace_operations()
                })
                .await
                .ok()
                .and_then(Result::ok);
                for index in included {
                    let request = &requests[index];
                    service.invalidate_accepted_history(request.conversation_id);
                    service.invalidate_history_snapshots(Some(request.conversation_id));
                    let cleanup_pending = pending
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&request.request_id));
                    results[index] = Some(Ok((
                        ConversationDeletionReceipt {
                            request_id: request.request_id,
                            conversation_id: request.conversation_id,
                        },
                        cleanup_pending,
                    )));
                }
                service.notify_deleted();
            }
            drop((owners, reservations, markers));
            results
                .into_iter()
                .map(|result| result.ok_or(Error::Catalog(CatalogError::Unavailable)))
                .collect()
        })
        .await
        .map_err(|_| Error::Catalog(CatalogError::Unavailable))?
    }
    pub async fn delete_conversation(
        self: &Arc<Self>,
        store: Arc<dyn ConversationDeletionStore>,
        request: ConversationDeletionRequest,
    ) -> Result<ConversationDeletionReceipt, Error> {
        self.delete_conversation_owned(store, request, None).await
    }
    pub(crate) async fn delete_conversation_in_epoch(
        self: &Arc<Self>,
        store: Arc<dyn ConversationDeletionStore>,
        request: ConversationDeletionRequest,
        expected: Uuid,
    ) -> Result<ConversationDeletionReceipt, Error> {
        if expected.is_nil() {
            return Err(CatalogError::InvalidInput.into());
        }
        self.delete_conversation_owned(store, request, Some(expected))
            .await
    }
    async fn delete_conversation_owned(
        self: &Arc<Self>,
        store: Arc<dyn ConversationDeletionStore>,
        request: ConversationDeletionRequest,
        expected: Option<Uuid>,
    ) -> Result<ConversationDeletionReceipt, Error> {
        request.validate()?;
        let conversation_id = request.conversation_id;
        let service = self.clone();
        {
            let mut deleting = service
                .deleting
                .lock()
                .map_err(|_| Error::Catalog(CatalogError::Unavailable))?;
            if !deleting.insert(conversation_id) {
                return Err(Error::Catalog(CatalogError::Busy));
            }
        }
        // Dropping an IPC waiter cannot detach the lock from an in-flight SQL commit.
        tokio::spawn(async move {
            let _marker = DeletionMarker {
                service: service.clone(),
                conversation_id,
            };
            async {
                let _admission = service
                    .tasks
                    .reserve_idle(request.conversation_id)
                    .map_err(|error| match error {
                        TaskRegistryError::Active | TaskRegistryError::Capacity => {
                            CatalogError::Busy
                        }
                        TaskRegistryError::Unavailable
                        | TaskRegistryError::Closing
                        | TaskRegistryError::SubtaskInvalid => CatalogError::Unavailable,
                    })?;
                let slot =
                    service
                        .slot(request.conversation_id, true)
                        .map_err(|error| match error {
                            AgentServiceError::Active | AgentServiceError::Capacity => {
                                CatalogError::Busy
                            }
                            _ => CatalogError::Unavailable,
                        })?;
                let mut owner = slot.lock().await;
                if owner.as_ref().is_some_and(|session| {
                    session.needs_recovery() || session.projection().active_turn().is_some()
                }) {
                    return Err(CatalogError::Busy.into());
                }
                // Release the idle writer before persistence. Failure requires a fresh
                // open, rather than continuing with a potentially deleted in-memory view.
                owner.take();
                // Keep the same slot registered and locked. Queued open/create/commit
                // callers cannot resurrect the old owner; durable tombstones fence creates.
                let receipt = if let Some(epoch) = expected {
                    store.delete_in_epoch(request, epoch).await?
                } else {
                    store.delete(request).await?
                };
                service.invalidate_accepted_history(conversation_id);
                service.invalidate_history_snapshots(Some(conversation_id));
                service.notify_deleted();
                Ok(receipt)
            }
            .await
        })
        .await
        .map_err(|_| Error::Catalog(CatalogError::Unavailable))?
    }
}

#[cfg(test)]
#[path = "deletion/tests.rs"]
mod tests;
