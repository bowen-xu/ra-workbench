//! Keep one existing session writer across every fresh legacy import batch.
use super::*;
use research_agent_core::protocol::EventPayload;
use tokio::sync::OwnedMutexGuard;

/// Holds the same slot across the import's post-write archive and completion.
pub(crate) struct LegacyImportGuard {
    _owner: OwnedMutexGuard<Option<DurableConversation>>,
}

impl AgentSessionService {
    /// Partial failures remain recovery-required; this does not authorize
    /// resuming an unproven prefix after restart.
    pub(crate) async fn commit_fresh_legacy_import(
        &self,
        id: ConversationId,
        payloads: &[EventPayload],
    ) -> Result<LegacyImportGuard, AgentServiceError> {
        let slot = self.slot(id, false)?;
        let mut owner = slot.lock_owned().await;
        let session = owner.as_mut().ok_or(AgentServiceError::NotOpen)?;
        if session.needs_recovery() || session.projection().seq() != 0 {
            return Err(SessionError::RecoveryRequired.into());
        }
        for chunk in payloads.chunks(450) {
            self.commit_payloads_owned(id, Uuid::new_v4(), chunk.to_vec(), session)
                .await?;
        }
        Ok(LegacyImportGuard { _owner: owner })
    }

    /// Serialize finalization with local writes, including when no live session
    /// is currently installed. The caller still verifies the durable journal.
    pub(crate) async fn hold_legacy_finalization(
        &self,
        id: ConversationId,
        expected: u64,
    ) -> Result<LegacyImportGuard, AgentServiceError> {
        let owner = self.slot(id, true)?.lock_owned().await;
        if owner.as_ref().is_some_and(|session| {
            session.needs_recovery() || session.projection().seq() != expected
        }) {
            return Err(SessionError::RecoveryRequired.into());
        }
        Ok(LegacyImportGuard { _owner: owner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::repository::SqliteConversationStore;
    use research_agent_core::{
        conversation::{HistoryPage, HistoryQuery, StoreFuture, StoreResult},
        protocol::{ModelIdentity, TurnPhase},
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    struct GatedStore {
        inner: Arc<SqliteConversationStore<AgentEvent>>,
        first: AtomicBool,
        entered: Notify,
        release: Notify,
    }
    impl ConversationStore<AgentEvent> for GatedStore {
        fn reserve_writer(&self, lease: WriterLease) -> StoreResult<()> {
            self.inner.reserve_writer(lease)
        }
        fn create(&self, lease: WriterLease) -> StoreFuture<'_, ()> {
            self.inner.create(lease)
        }
        fn resume(&self, lease: WriterLease) -> StoreFuture<'_, ()> {
            self.inner.resume(lease)
        }
        fn append(&self, lease: WriterLease, items: Vec<AgentEvent>) -> StoreFuture<'_, ()> {
            self.inner.append(lease, items)
        }
        fn persist(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
            self.inner.persist(lease)
        }
        fn flush(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
            Box::pin(async move {
                let watermark = self.inner.flush(lease).await?;
                if self.first.swap(false, Ordering::SeqCst) {
                    self.entered.notify_one();
                    self.release.notified().await;
                }
                Ok(watermark)
            })
        }
        fn shutdown(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
            self.inner.shutdown(lease)
        }
        fn discard(&self, lease: WriterLease) {
            self.inner.discard(lease);
        }
        fn history(&self, query: HistoryQuery) -> StoreFuture<'_, HistoryPage<AgentEvent>> {
            self.inner.history(query)
        }
    }

    #[tokio::test]
    async fn fresh_import_holds_the_writer_across_batches_and_refuses_a_nonempty_prefix() {
        let mut sql = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut sql, 67);
        let inner = Arc::new(
            SqliteConversationStore::new(Arc::new(Mutex::new(sql)), "test".into()).unwrap(),
        );
        let gated = Arc::new(GatedStore {
            inner,
            first: AtomicBool::new(true),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let service = Arc::new(AgentSessionService::new(gated.clone()));
        let id = ConversationId::from_uuid(Uuid::new_v4());
        service.create(id).await.unwrap();
        let turn = Uuid::new_v4();
        let mut payloads = vec![EventPayload::TurnStarted {
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "test".into(),
                revision: None,
            },
        }];
        payloads.extend((0..450).map(|_| EventPayload::PhaseChanged {
            turn_id: turn,
            phase: TurnPhase::RunningModel,
        }));
        let importing = {
            let service = service.clone();
            tokio::spawn(async move { service.commit_fresh_legacy_import(id, &payloads).await })
        };
        gated.entered.notified().await;
        let slot = service.slot(id, false).unwrap();
        assert!(slot.try_lock().is_err());
        let waiting = Arc::new(Notify::new());
        let competitor = {
            let service = service.clone();
            let waiting = waiting.clone();
            tokio::spawn(async move {
                waiting.notify_one();
                let mut owner = slot.lock().await;
                service
                    .commit_payloads_owned(
                        id,
                        Uuid::new_v4(),
                        vec![EventPayload::PhaseChanged {
                            turn_id: turn,
                            phase: TurnPhase::RunningModel,
                        }],
                        owner.as_mut().unwrap(),
                    )
                    .await
            })
        };
        waiting.notified().await;
        gated.release.notify_one();
        let guard = importing.await.unwrap().unwrap();
        assert!(!competitor.is_finished());
        drop(guard);
        assert_eq!(competitor.await.unwrap().unwrap().watermark, 452);
        assert!(service.hold_legacy_finalization(id, 451).await.is_err());
        let finalizing = service.hold_legacy_finalization(id, 452).await.unwrap();
        assert!(service.slot(id, false).unwrap().try_lock().is_err());
        drop(finalizing);
        assert!(service.commit_fresh_legacy_import(id, &[]).await.is_err());
        assert_eq!(
            service.read_since(id, 0, 500).await.unwrap().durable_count,
            452
        );
    }
}
