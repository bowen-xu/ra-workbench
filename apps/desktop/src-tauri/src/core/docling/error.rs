use serde::Serialize;
use thiserror::Error;

use crate::storage::StorageError;

#[derive(Debug, Error)]
pub(crate) enum DoclingError {
    #[error("Docling 解析已终止")]
    Cancelled,
    #[error("Docling 模型资源不可用：{0}")]
    Models(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("Docling 数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("Docling 服务配置无效：{0}")]
    InvalidSettings(String),
    #[error("Docling 服务未启用")]
    Disabled,
    #[error("当前客户端已断开 Docling 服务")]
    Disconnected,
    #[error("Docling 服务未连接，请在设置 → 本地服务中手动连接／启动 Docling")]
    ManualStartRequired,
    #[error("无法定位应用数据目录：{0}")]
    AppData(#[from] tauri::Error),
    #[error("Docling 服务文件操作失败：{0}")]
    File(#[from] std::io::Error),
    #[error("无法启动 Docling 服务：{0}")]
    Start(String),
    #[error("Docling 服务启动超时")]
    StartupTimeout,
    #[error("Docling 服务请求失败：{0}")]
    Request(String),
    #[error(
        "Docling 解析连续 {timeout_ms} 毫秒未收到有效任务状态响应（任务 ID：{request_id}）；最后状态：{last_error}"
    )]
    ResponseTimeout {
        timeout_ms: u64,
        request_id: String,
        last_error: String,
    },
    #[error("Docling 服务 registry 无效：{0}")]
    InvalidRegistry(String),
    #[error("Docling 服务协议不兼容：{0}")]
    Incompatible(String),
    #[error("Docling 服务无响应：{0}")]
    Unresponsive(String),
    #[error("Docling 返回了无效响应：{0}")]
    InvalidResponse(String),
    #[error("找不到 Docling 解析记录")]
    ParseRunNotFound,
    #[error("无法读取 PDF 附件：{0}")]
    Attachment(String),
}

impl From<reqwest::Error> for DoclingError {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error.to_string())
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for DoclingError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::Request(error.to_string())
    }
}

impl From<serde_json::Error> for DoclingError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidResponse(error.to_string())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingCommandError {
    code: &'static str,
    message: String,
}

impl From<DoclingError> for DoclingCommandError {
    fn from(error: DoclingError) -> Self {
        Self::from(&error)
    }
}

impl From<&DoclingError> for DoclingCommandError {
    fn from(error: &DoclingError) -> Self {
        let code = match error {
            DoclingError::Cancelled => "cancelled",
            DoclingError::InvalidSettings(_) => "invalid_settings",
            DoclingError::Models(_) => "models_unavailable",
            DoclingError::Disabled => "disabled",
            DoclingError::Disconnected => "disconnected",
            DoclingError::ManualStartRequired => "manual_start_required",
            DoclingError::StartupTimeout | DoclingError::Start(_) => "start_failed",
            DoclingError::Request(_) => "request_failed",
            DoclingError::ResponseTimeout { .. } => "response_timeout",
            DoclingError::InvalidRegistry(_) => "invalid_registry",
            DoclingError::Incompatible(_) => "incompatible",
            DoclingError::Unresponsive(_) => "unresponsive",
            DoclingError::InvalidResponse(_) => "invalid_response",
            DoclingError::ParseRunNotFound => "not_found",
            DoclingError::Attachment(_) => "attachment_error",
            DoclingError::Storage(_)
            | DoclingError::Database(_)
            | DoclingError::AppData(_)
            | DoclingError::File(_) => "storage_error",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<StorageError> for DoclingCommandError {
    fn from(error: StorageError) -> Self {
        if let StorageError::QueryWorker(cause) = error {
            return Self {
                code: "db_query_worker_failed",
                message: format!("解析记录查询工作线程失败：{cause}"),
            };
        }
        DoclingError::Storage(error).into()
    }
}

impl From<rusqlite::Error> for DoclingCommandError {
    fn from(error: rusqlite::Error) -> Self {
        DoclingError::Database(error).into()
    }
}

impl DoclingCommandError {
    pub(super) fn is_cancelled(&self) -> bool {
        self.code == "cancelled"
    }
}
