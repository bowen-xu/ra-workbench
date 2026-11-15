//! Fixed native stages supplied by App; no connection, root or process getter.
use super::model::{
    PendingEmbeddingJob, ReconciliationCounts, ReconciliationGeneration, ReconciliationRecord,
    ReconciliationSummary, SemanticIndexError,
};
use crate::semantic_engine::SemanticNamespace;
use crate::{
    ai::{EmbeddingAccess, PreparedEmbedding},
    semantic_engine::SemanticVectorStore,
};
pub(crate) trait IndexHost: Sync {
    type Error: From<SemanticIndexError> + Send;
    fn error_code(error: &Self::Error) -> &'static str;
    fn is_canceled(error: &Self::Error) -> bool;
    fn decide(
        &self,
        input: research_workbench_native_sdk::semantic_index::rules::IndexInputV1,
    ) -> impl Future<
        Output = Result<
            research_workbench_native_sdk::semantic_index::rules::IndexDecisionV1,
            Self::Error,
        >,
    > + Send;
    type Vector: SemanticVectorStore + Send + Sync;
    fn open(
        &self,
        relative: &str,
        namespace: SemanticNamespace,
        dimension: usize,
    ) -> impl Future<Output = Result<Self::Vector, Self::Error>> + Send;
    fn prepare_job(
        &self,
        embedding: &EmbeddingAccess,
        id: &str,
    ) -> impl Future<Output = Result<(PendingEmbeddingJob, PreparedEmbedding), Self::Error>> + Send;
    fn load_and_mark_running(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<PendingEmbeddingJob, Self::Error>> + Send;
    fn ready(
        &self,
        job: PendingEmbeddingJob,
        version: u64,
        norm: f32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn interrupt(&self, id: &str) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn reconciliation_targets(
        &self,
    ) -> impl Future<Output = Result<Vec<ReconciliationGeneration>, Self::Error>> + Send;
    fn complete_write(
        &self,
        record: ReconciliationRecord,
        version: u64,
        norm: f32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn repair_missing(
        &self,
        record: ReconciliationRecord,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn finalize(
        &self,
        id: &str,
        status: &'static str,
        counts: ReconciliationCounts,
        error: Option<&'static str>,
    ) -> impl Future<Output = Result<ReconciliationSummary, Self::Error>> + Send;
}
