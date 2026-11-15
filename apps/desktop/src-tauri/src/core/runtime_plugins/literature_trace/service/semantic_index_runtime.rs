//! The existing literature lifetime drains the original Core index worker.
use super::*;
use crate::runtime_plugins::literature_index::{host::IndexHost, reconciler};
use crate::{
    ai::EmbeddingAccess,
    runtime_plugins::literature_index::{
        IndexError, IndexOperation, IndexOutput, IndexResources, RuntimeIndexHost,
        manager::{self, SemanticIndexManager},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::semantic_index::{EmbeddingJobSummary, ReconciliationSummary};

impl LiteratureTraceService {
    pub(crate) fn semantic_index_backfill_in(
        &self,
        db: &rusqlite::Connection,
        key: &str,
    ) -> Result<i64, IndexError> {
        self.available()?;
        let package = package::accept_semantic_index(&self.catalog, key)?;
        let resources = IndexResources::load(&self.catalog, &package)
            .map_err(|error| Self::error(error.code, error.message).package(key))?;
        crate::runtime_plugins::literature_index::backfill_in(db, &resources)
    }
    pub(crate) async fn reconcile_semantic_index(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        manager: SemanticIndexManager,
    ) -> Result<ReconciliationSummary, IndexError> {
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            let _lifetime = owner.doi_operations.read().await;
            let host = owner.index_host(store, key).await?;
            let IndexOutput::RunId(id) = host.run(IndexOperation::BeginReconciliation).await?
            else {
                return Err(IndexError::mismatch());
            };
            let registration = match manager.register_owned(&id) {
                Ok(value) => value,
                Err(error) => {
                    host.finalize(&id, "failed", Default::default(), Some("worker_busy"))
                        .await?;
                    return Err(error.into());
                }
            };
            let signal = registration.signal();
            let stopped = owner.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            if owner.stopping.load(Ordering::Acquire) {
                signal.cancel();
            }
            let work = reconciler::reconcile(&host, &id, &signal);
            tokio::pin!(work);
            let result = tokio::select! { biased;
                result = &mut work => result,
                _ = &mut stopped => { signal.cancel(); work.await },
            };
            drop(registration);
            result
        })
        .await
        .map_err(|_| IndexError::Unsettled)?
    }
    async fn index_host(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
    ) -> Result<RuntimeIndexHost, IndexError> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_semantic_index(&catalog, &selected)?;
            IndexResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| IndexError::Unsettled)??;
        Ok(RuntimeIndexHost::new(
            Arc::clone(self),
            store,
            key,
            Arc::new(resources),
        ))
    }
    pub(crate) async fn semantic_index_request(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        operation: IndexOperation,
    ) -> Result<IndexOutput, IndexError> {
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            let _lifetime = owner.doi_operations.read().await;
            owner.index_host(store, key).await?.run(operation).await
        })
        .await
        .map_err(|_| IndexError::Unsettled)?
    }
    pub(crate) async fn start_semantic_index_job(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        embedding: EmbeddingAccess,
        manager: SemanticIndexManager,
        representation: String,
    ) -> Result<EmbeddingJobSummary, IndexError> {
        let owner = Arc::clone(self);
        let (send, receive) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _lifetime = owner.doi_operations.read().await;
            let started = async {
                let host = owner.index_host(store, key).await?;
                let IndexOutput::Retry(summary, should_start) =
                    host.run(IndexOperation::Retry(representation)).await?
                else {
                    return Err(IndexError::mismatch());
                };
                let registration = if should_start {
                    Some(manager.register_owned(&summary.job_id)?)
                } else {
                    None
                };
                Ok::<_, IndexError>((host, summary, registration))
            }
            .await;
            let (host, summary, registration) = match started {
                Ok(value) => value,
                Err(error) => {
                    drop(send.send(Err(error)));
                    return;
                }
            };
            let id = summary.job_id.clone();
            drop(send.send(Ok(summary)));
            let Some(registration) = registration else {
                return;
            };
            let signal = registration.signal();
            let stopped = owner.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            if owner.stopping.load(Ordering::Acquire) {
                signal.cancel();
            }
            let work = manager::execute_job(&host, &embedding, &id, &signal);
            tokio::pin!(work);
            let result = tokio::select! { biased;
                result = &mut work => result,
                _ = &mut stopped => { signal.cancel(); work.await },
            };
            if let Err(error) = result
                && !error.needs_recovery()
            {
                // Same original terminal updates. The accepted owner still holds
                // its lifetime and manager registration through their completion.
                let _ = host.fail_job(id.clone(), &error).await;
            }
            drop(registration);
        });
        receive.await.map_err(|_| IndexError::Unsettled)?
    }
}
