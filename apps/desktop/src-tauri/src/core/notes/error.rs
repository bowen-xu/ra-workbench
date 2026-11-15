use serde::Serialize;
use thiserror::Error;

use crate::storage::StorageError;

#[derive(Debug, Error)]
pub(crate) enum NotesError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("笔记数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("笔记文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("笔记输入无效：{0}")]
    InvalidInput(String),
    #[error("找不到笔记对象")]
    NotFound,
    #[error("笔记已被其他操作更新，当前版本为 {current_revision}")]
    RevisionConflict { current_revision: i64 },
    #[error("note_catalog_changed：笔记目录已更新，请重新读取列表。")]
    CatalogChanged { current_revision: i64 },
    #[error("note_catalog_preparing：笔记搜索元数据正在按批准备。")]
    CatalogPreparing,
    #[error("笔记索引正在重建")]
    IndexBusy,
    #[error("笔记索引重建已取消")]
    IndexCancelled,
    #[error("笔记索引操作失败：{0}")]
    IndexFailed(String),
    #[error("笔记传输任务正在运行")]
    TransferBusy,
    #[error("笔记操作已取消")]
    OperationCancelled,
    #[error("笔记传输失败：{0}")]
    TransferFailed(String),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesCommandError {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<i64>,
}

impl From<NotesError> for NotesCommandError {
    fn from(error: NotesError) -> Self {
        let (code, current_revision) = match &error {
            NotesError::InvalidInput(_) => ("invalid_input", None),
            NotesError::NotFound => ("not_found", None),
            NotesError::RevisionConflict { current_revision }
            | NotesError::CatalogChanged { current_revision } => {
                ("revision_conflict", Some(*current_revision))
            }
            NotesError::CatalogPreparing => ("catalog_preparing", None),
            NotesError::IndexBusy => ("index_busy", None),
            NotesError::IndexCancelled => ("operation_cancelled", None),
            NotesError::IndexFailed(_) => ("index_failed", None),
            NotesError::TransferBusy | NotesError::TransferFailed(_) => ("transfer_failed", None),
            NotesError::OperationCancelled => ("operation_cancelled", None),
            NotesError::Storage(StorageError::QueryWorker(_)) => ("db_query_worker_failed", None),
            NotesError::Storage(StorageError::DatabaseBusy) => ("database_busy", None),
            NotesError::Storage(_) | NotesError::Database(_) | NotesError::Io(_) => {
                ("storage_error", None)
            }
        };
        Self {
            code,
            message: error.to_string(),
            current_revision,
        }
    }
}

impl From<rusqlite::Error> for NotesCommandError {
    fn from(error: rusqlite::Error) -> Self {
        NotesError::Database(error).into()
    }
}

impl From<StorageError> for NotesCommandError {
    fn from(error: StorageError) -> Self {
        NotesError::Storage(error).into()
    }
}

impl From<std::io::Error> for NotesCommandError {
    fn from(error: std::io::Error) -> Self {
        NotesError::Io(error).into()
    }
}
