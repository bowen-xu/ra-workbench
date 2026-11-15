//! Pure shared retrieval/failure contracts, independent of plugin repositories.
use crate::storage::StorageError;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum SemanticIndexError {
    #[error("语义向量索引输入无效：{0}")]
    InvalidInput(&'static str),
    #[error("必需的语义引擎动态库无法加载：{0}")]
    EngineLoad(String),
    #[error("语义引擎 ABI 不兼容")]
    EngineAbiMismatch,
    #[error("语义引擎协议响应无效：{0}")]
    EngineProtocol(String),
    #[error("语义引擎操作失败：{code}")]
    EngineOperation { code: String },
    #[error("语义向量索引控制平面数据库操作失败")]
    Database(#[from] rusqlite::Error),
    #[error("语义向量索引本地存储不可用")]
    Storage(#[from] StorageError),
    #[error("语义向量索引后台任务不可用")]
    WorkerUnavailable,
    #[error("语义向量任务已取消")]
    Canceled,
    #[error("embedding provider 调用失败：{0}")]
    Provider(&'static str),
}

#[derive(Debug, Error)]
pub(crate) enum KnowledgeError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("知识数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("语义索引控制状态保存失败")]
    SemanticIndex(#[from] SemanticIndexError),
    #[error("{0}")]
    InvalidInput(String),
    #[error("找不到研究对象或来源锚点")]
    NotFound,
    #[error("对象版本冲突；当前版本为 {current_revision}")]
    RevisionConflict { current_revision: i64 },
}

#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "preview context remains a compatibility boundary for paper-reading migration"
)]
pub(crate) struct ConceptRetrievalContext {
    pub trace_id: String,
    pub context_text: String,
}

#[derive(Debug, Error)]
#[allow(
    dead_code,
    reason = "preview lifecycle errors remain part of the compatibility boundary"
)]
pub(crate) enum ConceptRetrievalError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("找不到概念网络检索预览")]
    NotFound,
    #[error("概念网络检索预览已过期，请重新预览")]
    Expired,
    #[error("概念网络检索预览与当前问题不匹配")]
    QueryMismatch,
    #[error("概念网络检索预览已经被使用")]
    AlreadyConsumed,
    #[error("概念网络数据库操作失败")]
    Database(#[from] rusqlite::Error),
    #[error("概念网络本地存储不可用")]
    Storage(#[from] StorageError),
    #[error("概念语义索引不可用")]
    SemanticIndex(#[from] SemanticIndexError),
    #[error("查询 embedding provider 暂时不可用")]
    Provider,
    #[error("概念网络检索后台任务不可用")]
    WorkerUnavailable,
    #[error("概念网络检索已取消")]
    Canceled,
}

impl From<crate::semantic_engine::VectorError> for SemanticIndexError {
    fn from(error: crate::semantic_engine::VectorError) -> Self {
        use crate::semantic_engine::VectorError as E;
        match error {
            E::InvalidInput(value) => Self::InvalidInput(value),
            E::EngineLoad(value) => Self::EngineLoad(value),
            E::EngineAbiMismatch => Self::EngineAbiMismatch,
            E::EngineProtocol(value) => Self::EngineProtocol(value),
            E::EngineOperation { code } => Self::EngineOperation { code },
            E::WorkerUnavailable => Self::WorkerUnavailable,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCommandError {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<i64>,
}

impl KnowledgeCommandError {
    pub(crate) fn into_native(self) -> Box<crate::runtime_plugins::native::NativeHostError> {
        use crate::runtime_plugins::native::{NativeHostError, NativeProfile};
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, self.code, self.message);
        error.current_revision = self.current_revision;
        error
    }
}

impl From<KnowledgeError> for KnowledgeCommandError {
    fn from(error: KnowledgeError) -> Self {
        let current_revision = match &error {
            KnowledgeError::RevisionConflict { current_revision } => Some(*current_revision),
            _ => None,
        };
        let message = match &error {
            KnowledgeError::Storage(_)
            | KnowledgeError::Database(_)
            | KnowledgeError::SemanticIndex(_) => {
                "知识数据保存失败，请重试；若持续失败请检查本地存储状态".to_owned()
            }
            _ => error.to_string(),
        };
        let code = match error {
            KnowledgeError::InvalidInput(_) => "invalid_input",
            KnowledgeError::NotFound => "not_found",
            KnowledgeError::RevisionConflict { .. } => "revision_conflict",
            KnowledgeError::Storage(_)
            | KnowledgeError::Database(_)
            | KnowledgeError::SemanticIndex(_) => "storage_error",
        };
        Self {
            code,
            message,
            current_revision,
        }
    }
}

impl From<StorageError> for KnowledgeCommandError {
    fn from(error: StorageError) -> Self {
        if let StorageError::QueryWorker(cause) = error {
            return Self {
                code: "db_query_worker_failed",
                message: format!("阅读记录查询工作线程失败：{cause}"),
                current_revision: None,
            };
        }
        KnowledgeError::Storage(error).into()
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum CaptureCommandError {
    Knowledge(KnowledgeCommandError),
    Technical(CaptureTechnicalError),
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureTechnicalError {
    #[serde(flatten)]
    error: super::error::AiCommandError,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<i64>,
}
impl From<super::error::AiError> for CaptureTechnicalError {
    fn from(error: super::error::AiError) -> Self {
        let current_revision = match &error {
            super::error::AiError::NativeWorkflow { failure, .. } => failure.current_revision,
            _ => None,
        };
        Self {
            error: error.into(),
            current_revision,
        }
    }
}
impl From<super::error::AiError> for CaptureCommandError {
    fn from(error: super::error::AiError) -> Self {
        match error {
            super::error::AiError::Knowledge(error) => Self::Knowledge(error.into()),
            other => Self::Technical(other.into()),
        }
    }
}

/// Original static JSON errors and direct Native diagnostics share one command boundary.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum KnowledgeRouteError {
    Native(Box<crate::runtime_plugins::native::NativeHostError>),
}
impl From<Box<crate::runtime_plugins::native::NativeHostError>> for KnowledgeRouteError {
    fn from(error: Box<crate::runtime_plugins::native::NativeHostError>) -> Self {
        Self::Native(error)
    }
}
