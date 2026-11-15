//! Shared metadata and physical error types used by the original Core driver.
pub(crate) use crate::ai::knowledge_contracts::SemanticIndexError;
pub(crate) use research_workbench_native_sdk::semantic_index::{
    PendingEmbeddingJob, ReconciliationCounts, ReconciliationGeneration, ReconciliationRecord,
    ReconciliationSummary,
};

use serde::Serialize;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticIndexCommandError {
    code: &'static str,
    message: &'static str,
}

impl From<SemanticIndexError> for SemanticIndexCommandError {
    fn from(error: SemanticIndexError) -> Self {
        let code = match error {
            SemanticIndexError::InvalidInput(_) => "invalid_input",
            SemanticIndexError::Canceled => "canceled",
            SemanticIndexError::Provider(_) => "provider_error",
            SemanticIndexError::EngineLoad(_)
            | SemanticIndexError::EngineAbiMismatch
            | SemanticIndexError::EngineProtocol(_)
            | SemanticIndexError::EngineOperation { .. } => "index_error",
            SemanticIndexError::Database(_) | SemanticIndexError::Storage(_) => "storage_error",
            SemanticIndexError::WorkerUnavailable => "worker_unavailable",
        };
        let message = match code {
            "invalid_input" => "语义索引请求无效或当前状态不可执行",
            "canceled" => "语义索引任务已取消",
            "provider_error" => "embedding provider 调用失败；可检查设置后重试",
            "index_error" => "必需的 LanceDB 语义引擎不可用；请修复组件后重试",
            "storage_error" => "语义索引控制状态保存失败；请检查本地存储",
            _ => "语义索引后台任务暂时不可用",
        };
        Self { code, message }
    }
}
