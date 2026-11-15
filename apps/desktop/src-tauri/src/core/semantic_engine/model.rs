use super::VectorError;
pub(crate) use research_semantic_engine_api::{
    SemanticNamespace, SemanticVectorRow, SemanticVectorSearchHit, StoredVectorMetadata,
    VectorWriteReceipt,
};
use std::future::Future;
pub(crate) trait SemanticVectorStore {
    fn ensure_ready(&self) -> impl Future<Output = Result<u64, VectorError>> + Send;

    fn upsert(
        &self,
        row: &SemanticVectorRow,
    ) -> impl Future<Output = Result<VectorWriteReceipt, VectorError>> + Send;

    fn contains(
        &self,
        row_id: &str,
        input_sha256: &str,
    ) -> impl Future<Output = Result<bool, VectorError>> + Send;

    fn list_metadata(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredVectorMetadata>, VectorError>> + Send;

    fn search(
        &self,
        query: &[f32],
        limit: usize,
    ) -> impl Future<Output = Result<Vec<SemanticVectorSearchHit>, VectorError>> + Send;

    fn delete(&self, row_id: &str) -> impl Future<Output = Result<u64, VectorError>> + Send;
}
