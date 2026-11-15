use serde::Serialize;
use thiserror::Error;

use crate::storage::StorageError;

#[derive(Debug, Error)]
pub(crate) enum ContentError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("内容存储数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("内容存储文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("内容垃圾回收任务正在运行")]
    Busy,
    #[error("内容垃圾回收已取消")]
    Cancelled,
    #[error("历史附件对账未完成，禁止删除内容对象：{0}")]
    BackfillBlocked(String),
    #[error("内容对象路径不安全或与哈希不一致")]
    UnsafePath,
}

impl ContentError {
    pub(crate) fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::Busy => "content_gc_busy",
            Self::Cancelled => "operation_cancelled",
            Self::BackfillBlocked(_) => "content_gc_blocked",
            Self::UnsafePath => "content_integrity_error",
            Self::Storage(_) | Self::Database(_) | Self::Io(_) => "storage_error",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentCommandError {
    code: &'static str,
    message: String,
}

impl From<ContentError> for ContentCommandError {
    fn from(error: ContentError) -> Self {
        let code = error.diagnostic_code();
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<StorageError> for ContentCommandError {
    fn from(error: StorageError) -> Self {
        ContentError::Storage(error).into()
    }
}
