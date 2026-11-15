//! Original C ABI vector store, bound to an App-selected controlled index root.
mod error;
mod model;
mod paths;
mod semantic_engine_store;
pub(crate) use error::VectorError;
pub(crate) use model::{
    SemanticNamespace, SemanticVectorRow, SemanticVectorSearchHit, SemanticVectorStore,
    StoredVectorMetadata, VectorWriteReceipt,
};
pub(crate) use semantic_engine_store::{SemanticEngineVectorStore, validate_required_engine};
use std::path::PathBuf;
#[derive(Clone)]
pub(crate) struct VectorAccess {
    root: PathBuf,
}
impl VectorAccess {
    /// Internal App composition selects its existing Core-owned index root.
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub(crate) async fn open(
        &self,
        relative: &str,
        namespace: SemanticNamespace,
        dimension: usize,
    ) -> Result<SemanticEngineVectorStore, VectorError> {
        let path = paths::controlled_database_path(&self.root, relative)?;
        SemanticEngineVectorStore::open(&path, namespace, dimension).await
    }
}
