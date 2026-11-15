//! Core owns the original export capture, cancellation and physical publication.
pub(crate) mod capture;
mod manager;
mod resources;
pub(crate) mod writer;
pub(crate) use manager::ResearchExportManager;
pub(crate) use resources::ResearchExportSql;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::storage::StorageError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchExportRequest {
    pub(crate) target_directory: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchExportReport {
    pub(crate) folder_name: String,
    pub(crate) file_count: usize,
    pub(crate) object_count: usize,
    pub(crate) source_count: usize,
}

#[derive(Debug, Error)]
pub(crate) enum ResearchExportError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("研究知识导出数据库读取失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("研究知识导出序列化失败：{0}")]
    Serialization(#[from] serde_json::Error),
    #[error("研究知识导出文件写入失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("所选导出目录不可用")]
    InvalidTarget,
    #[error("同名导出目录已存在，不会覆盖")]
    DestinationExists,
    #[error("研究知识导出超过 100,000 条记录或 64 MiB 安全上限")]
    TooLarge,
    #[error("已有研究知识导出正在运行")]
    Busy,
    #[error("研究知识导出已取消，未生成不完整目录")]
    Cancelled,
    #[error("研究知识导出任务异常终止")]
    TaskFailed,
    #[error("研究知识导出数据集范围不匹配")]
    InvalidSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchExportCommandError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl From<ResearchExportError> for ResearchExportCommandError {
    fn from(error: ResearchExportError) -> Self {
        let code = match error {
            ResearchExportError::InvalidTarget => "invalid_target",
            ResearchExportError::DestinationExists => "destination_exists",
            ResearchExportError::TooLarge => "too_large",
            ResearchExportError::Busy => "busy",
            ResearchExportError::Cancelled => "operation_cancelled",
            ResearchExportError::Storage(_)
            | ResearchExportError::Database(_)
            | ResearchExportError::Serialization(_)
            | ResearchExportError::Io(_)
            | ResearchExportError::TaskFailed
            | ResearchExportError::InvalidSnapshot => "export_failed",
        };
        let message = match &error {
            ResearchExportError::Storage(_)
            | ResearchExportError::Database(_)
            | ResearchExportError::Serialization(_)
            | ResearchExportError::Io(_)
            | ResearchExportError::TaskFailed
            | ResearchExportError::InvalidSnapshot => {
                "研究知识导出失败；原始资料未改变，请检查目标目录后重试".to_owned()
            }
            _ => error.to_string(),
        };
        Self { code, message }
    }
}
