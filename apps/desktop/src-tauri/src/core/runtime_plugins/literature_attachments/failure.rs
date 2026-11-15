//! Preserve safe original causes and the exact accepted write prefix.
use crate::{
    content::{PdfObjectError, StoredObjectReadError},
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
    storage::StorageError,
};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AttachmentSql {
    NotStarted,
    StatementFailed,
    Returned,
    RolledBack,
    Committed,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentProgress {
    pub(crate) phase: &'static str,
    pub(crate) sql: AttachmentSql,
    pub(crate) attachment_id: Option<String>,
    pub(crate) content_publish_returned: bool,
    pub(crate) attachment_insert_returned: bool,
    pub(crate) duplicate_insert_returns: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentFailure {
    #[serde(flatten)]
    pub(crate) progress: Box<AttachmentProgress>,
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Box<PluginDataError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) database: Option<Box<SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback: Option<Box<SafeDatabaseError>>,
}

impl AttachmentProgress {
    pub(crate) fn new(phase: &'static str) -> Self {
        Self {
            phase,
            sql: AttachmentSql::NotStarted,
            attachment_id: None,
            content_publish_returned: false,
            attachment_insert_returned: false,
            duplicate_insert_returns: 0,
        }
    }
    pub(crate) fn fail(&self, code: &str, message: impl Into<String>) -> AttachmentFailure {
        AttachmentFailure {
            progress: Box::new(self.clone()),
            code: code.into(),
            message: message.into(),
            data: None,
            database: None,
            rollback: None,
        }
    }
    pub(crate) fn missing(&self) -> AttachmentFailure {
        self.fail("not_found", "找不到文献或附件")
    }
    pub(crate) fn sql_error(&self, cause: &rusqlite::Error) -> AttachmentFailure {
        let safe = SafeDatabaseError::sql(self.phase, cause);
        let mut failure = self.fail("storage_error", format!("文献数据库操作失败：{safe}"));
        failure.database = Some(Box::new(safe));
        failure
    }
    pub(crate) fn data(&self, cause: Box<PluginDataError>) -> AttachmentFailure {
        // Original SQL text belongs only to the existing persisted import
        // result, never to a runtime diagnostic or model-facing error.
        let mut failure = self.fail(
            "storage_error",
            format!("文献数据库操作失败：{}", cause.safe_cause),
        );
        failure.data = Some(cause);
        failure
    }
    pub(crate) fn io(&self, cause: std::io::Error) -> AttachmentFailure {
        self.fail("storage_error", format!("文件操作失败：{cause}"))
    }
    pub(crate) fn stored(&self, cause: StoredObjectReadError) -> AttachmentFailure {
        match cause {
            StoredObjectReadError::Io(cause) => self.io(cause),
            StoredObjectReadError::MissingFile => self.fail("not_found", "附件文件已不存在"),
            StoredObjectReadError::UnsafePath => self.fail("storage_error", "附件存储路径不安全"),
        }
    }
    pub(crate) fn pdf(&self, cause: PdfObjectError) -> AttachmentFailure {
        match cause {
            PdfObjectError::Io(cause) => self.io(cause),
            PdfObjectError::Database(cause) => self.sql_error(&cause),
            PdfObjectError::InvalidPdf(path) => {
                self.fail("invalid_input", format!("不是有效的 PDF 文件：{path}"))
            }
            PdfObjectError::UnsafeStoredObject => self.fail("storage_error", "附件存储路径不安全"),
        }
    }
    pub(crate) fn owner(&self, cause: StorageError) -> AttachmentFailure {
        use crate::data_interface::DataError;
        match cause {
            StorageError::Database(cause) | StorageError::DataInterface(DataError::Sql(cause)) => {
                self.sql_error(&cause)
            }
            StorageError::DataInterface(cause) => {
                let category = match cause {
                    DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
                    DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
                    DataError::Log(_) => "DATA_RUNTIME_LOG",
                    DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
                    DataError::Sql(_) => unreachable!("SQL was handled above"),
                };
                self.fail("storage_error", format!("Core数据拥有者不可用：{category}"))
            }
            StorageError::QueryWorker(_) => {
                self.fail("db_query_worker_failed", "附件查询工作线程未结算。")
            }
            cause => self.fail("storage_error", cause.to_string()),
        }
    }
}

impl AttachmentFailure {
    pub(crate) fn native(self) -> Box<NativeHostError> {
        let message = if self.progress.sql == AttachmentSql::Unknown {
            format!("附件操作结果未知，禁止自动重试。{}", self.message)
        } else if self.progress.sql == AttachmentSql::Committed {
            format!(
                "附件操作已有提交，请刷新核对，禁止自动重放。{}",
                self.message
            )
        } else if self.progress.attachment_insert_returned {
            format!(
                "附件写入已返回，可能已有持久前缀，请刷新核对，禁止自动重放。{}",
                self.message
            )
        } else if self.progress.phase == "content-publish" {
            format!(
                "附件文件可能已写入，文件发布未完整返回，请刷新核对，禁止自动重放。{}",
                self.message
            )
        } else {
            self.message.clone()
        };
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, &self.code, message);
        error.data_error = self.data.clone();
        error.attachments = Some(Box::new(self));
        error
    }
    pub(crate) fn unknown(phase: &'static str) -> Self {
        let mut progress = AttachmentProgress::new(phase);
        progress.sql = AttachmentSql::Unknown;
        progress.fail("db_query_worker_failed", "附件工作线程未结算。")
    }
}
