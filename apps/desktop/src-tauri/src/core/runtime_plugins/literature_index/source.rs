//! Original static App callbacks. No runtime package can construct these ports.
use super::{manager::SemanticIndexManager, model::SemanticIndexCommandError};
use crate::{ai::EmbeddingAccess, storage::AppStore};
use research_workbench_native_sdk::semantic_index::{
    EmbeddingJobSummary, IndexGenerationSummary, ReconciliationSummary, SemanticIndexStatus,
};
use rusqlite::Connection;
use std::{future::Future, pin::Pin};

type ReconciliationFuture =
    Pin<Box<dyn Future<Output = Result<ReconciliationSummary, SemanticIndexCommandError>> + Send>>;
#[derive(Clone, Copy)]
pub(crate) struct SemanticIndexSource {
    pub status: fn(&mut Connection) -> Result<SemanticIndexStatus, SemanticIndexCommandError>,
    pub job: fn(&mut Connection, String) -> Result<EmbeddingJobSummary, SemanticIndexCommandError>,
    pub retry: fn(
        AppStore,
        EmbeddingAccess,
        SemanticIndexManager,
        String,
    ) -> Result<EmbeddingJobSummary, SemanticIndexCommandError>,
    pub rebuild:
        fn(&mut Connection, String) -> Result<IndexGenerationSummary, SemanticIndexCommandError>,
    pub reconcile: fn(AppStore, SemanticIndexManager) -> ReconciliationFuture,
}
