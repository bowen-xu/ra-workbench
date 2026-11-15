//! Core executes the original fixed index port for an explicitly accepted package.
use super::{
    execute::{self, Operation, Output},
    failure::Error,
    host::IndexHost,
    model::*,
    resources::IndexResources,
};
use crate::{
    ai::{EmbeddingAccess, PreparedEmbedding},
    runtime_plugins::literature_trace::service::LiteratureTraceService,
    semantic_engine::{SemanticEngineVectorStore, SemanticNamespace, VectorAccess},
    storage::AppStore,
};
use research_workbench_native_sdk::semantic_index::rules::{IndexDecisionV1, IndexInputV1};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct RuntimeIndexHost {
    owner: Arc<LiteratureTraceService>,
    store: AppStore,
    key: String,
    resources: Arc<IndexResources>,
}
impl RuntimeIndexHost {
    pub(crate) fn new(
        owner: Arc<LiteratureTraceService>,
        store: AppStore,
        key: String,
        resources: Arc<IndexResources>,
    ) -> Self {
        Self {
            owner,
            store,
            key,
            resources,
        }
    }
    pub(crate) async fn run(&self, operation: Operation) -> Result<Output, Error> {
        execute::run(
            &self.owner,
            self.store.clone(),
            &self.key,
            Arc::clone(&self.resources),
            operation,
        )
        .await
    }
    async fn complete(&self, operation: Operation) -> Result<(), Error> {
        match self.run(operation).await? {
            Output::Complete => Ok(()),
            _ => Err(Error::mismatch()),
        }
    }
    pub(crate) async fn fail_job(&self, id: String, error: &Error) -> Result<(), Error> {
        self.complete(Operation::Failed {
            id,
            code: Self::error_code(error),
            canceled: Self::is_canceled(error),
        })
        .await
    }
}
impl IndexHost for RuntimeIndexHost {
    type Error = Error;
    type Vector = SemanticEngineVectorStore;
    fn error_code(error: &Error) -> &'static str {
        match error {
            Error::Original(error) => super::manager::error_code(error),
            Error::Rejected { .. } | Error::Conflict(_) => "invalid_input",
            Error::Native { .. } => "native_plugin_failure",
            Error::Boundary(failure) => Self::error_code(&failure.failure.error),
            Error::Unsettled | Error::Need(_) => "storage_error",
        }
    }
    fn is_canceled(error: &Error) -> bool {
        match error {
            Error::Original(SemanticIndexError::Canceled) => true,
            Error::Boundary(failure) => Self::is_canceled(&failure.failure.error),
            _ => false,
        }
    }
    async fn decide(&self, input: IndexInputV1) -> Result<IndexDecisionV1, Error> {
        self.owner
            .semantic_index_rule(&self.key, input)
            .await
            .map(|(decision, _)| decision)
            .map_err(Into::into)
    }
    async fn open(
        &self,
        relative: &str,
        namespace: SemanticNamespace,
        dimension: usize,
    ) -> Result<Self::Vector, Error> {
        VectorAccess::new(self.store.semantic_index_directory().to_path_buf())
            .open(relative, namespace, dimension)
            .await
            .map_err(|error| SemanticIndexError::from(error).into())
    }
    async fn prepare_job(
        &self,
        embedding: &EmbeddingAccess,
        id: &str,
    ) -> Result<(PendingEmbeddingJob, PreparedEmbedding), Error> {
        match self
            .run(Operation::Prepare {
                id: id.into(),
                embedding: embedding.clone(),
            })
            .await?
        {
            Output::Prepared(value) => Ok(*value),
            _ => Err(Error::mismatch()),
        }
    }
    async fn load_and_mark_running(&self, id: &str) -> Result<PendingEmbeddingJob, Error> {
        match self
            .run(Operation::Running {
                id: id.into(),
                lease: "semantic-index-worker",
            })
            .await?
        {
            Output::Running(value) => Ok(*value),
            _ => Err(Error::mismatch()),
        }
    }
    async fn ready(&self, job: PendingEmbeddingJob, version: u64, norm: f32) -> Result<(), Error> {
        self.complete(Operation::Ready {
            job: Box::new(job),
            version,
            norm,
        })
        .await
    }
    async fn interrupt(&self, id: &str) -> Result<(), Error> {
        self.complete(Operation::Interrupt(id.into())).await
    }
    async fn reconciliation_targets(&self) -> Result<Vec<ReconciliationGeneration>, Error> {
        match self.run(Operation::ReconciliationTargets).await? {
            Output::Targets(value) => Ok(value),
            _ => Err(Error::mismatch()),
        }
    }
    async fn complete_write(
        &self,
        record: ReconciliationRecord,
        version: u64,
        norm: f32,
    ) -> Result<(), Error> {
        let Output::Retry(summary, _) =
            self.run(Operation::Retry(record.representation_id)).await?
        else {
            return Err(Error::mismatch());
        };
        let Output::Running(job) = self
            .run(Operation::Running {
                id: summary.job_id,
                lease: "semantic-index-reconciler",
            })
            .await?
        else {
            return Err(Error::mismatch());
        };
        self.ready(*job, version, norm).await
    }
    async fn repair_missing(&self, record: ReconciliationRecord) -> Result<(), Error> {
        let IndexDecisionV1::Flag { value: stale } = self
            .decide(IndexInputV1::RepairState {
                status: record.status.clone(),
            })
            .await?
        else {
            return Err(Error::mismatch());
        };
        if stale {
            self.complete(Operation::MarkStale(record.clone())).await?;
        }
        match self.run(Operation::Retry(record.representation_id)).await? {
            Output::Retry(_, _) => Ok(()),
            _ => Err(Error::mismatch()),
        }
    }
    async fn finalize(
        &self,
        id: &str,
        status: &'static str,
        counts: ReconciliationCounts,
        error: Option<&'static str>,
    ) -> Result<ReconciliationSummary, Error> {
        match self
            .run(Operation::FinishReconciliation {
                id: id.into(),
                status,
                counts,
                error,
            })
            .await?
        {
            Output::Reconciled(value) => Ok(value),
            _ => Err(Error::mismatch()),
        }
    }
}
