use crate::{
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::native::{NativeHostError, SafeDatabaseError},
    storage::StorageError,
};
use research_workbench_native_sdk::browser_capture::CaptureRejectionV1;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CaptureFailure {
    #[error("Invalid browser capture request")]
    InvalidInput,
    #[error("Capture identity already contains different data")]
    Conflict,
    #[error("Capture session or active parent item was not found")]
    NotFound,
    #[error("This connector capability is not supported")]
    Unsupported,
    #[error("Browser capture could not be stored")]
    Storage,
    #[error("Browser capture outcome is unknown; {cause}")]
    Unknown {
        cause: Box<CaptureFailure>,
        rollback: Option<Box<SafeDatabaseError>>,
    },
    #[error("Browser capture could not be stored; {0}")]
    Database(Box<SafeDatabaseError>),
    #[error("Browser capture could not be stored; {0}")]
    Data(Box<PluginDataError>),
    #[error("{0}")]
    Native(Box<NativeHostError>),
    #[error("Browser capture could not be stored; {0}")]
    Technical(String),
}
impl CaptureFailure {
    pub(crate) fn code(&self) -> &str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::Conflict => "capture_conflict",
            Self::NotFound => "not_found",
            Self::Unsupported => "unsupported",
            Self::Native(error) => &error.code,
            _ => "storage_error",
        }
    }
    pub(crate) fn status(&self) -> u16 {
        match self {
            Self::InvalidInput => 400,
            Self::Conflict => 409,
            Self::NotFound => 404,
            Self::Unsupported => 501,
            Self::Native(error) => match error.code.as_str() {
                "invalid_input" => 400,
                "capture_conflict" => 409,
                "not_found" => 404,
                "unsupported" => 501,
                _ => 500,
            },
            _ => 500,
        }
    }
}
impl From<CaptureRejectionV1> for CaptureFailure {
    fn from(value: CaptureRejectionV1) -> Self {
        match value {
            CaptureRejectionV1::InvalidInput => Self::InvalidInput,
            CaptureRejectionV1::Unsupported => Self::Unsupported,
        }
    }
}
impl From<rusqlite::Error> for CaptureFailure {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(Box::new(SafeDatabaseError::sql("browser-capture", &value)))
    }
}
impl From<StorageError> for CaptureFailure {
    fn from(_: StorageError) -> Self {
        Self::Storage
    }
}
impl From<Box<PluginDataError>> for CaptureFailure {
    fn from(value: Box<PluginDataError>) -> Self {
        Self::Data(value)
    }
}
impl From<Box<NativeHostError>> for CaptureFailure {
    fn from(value: Box<NativeHostError>) -> Self {
        Self::Native(value)
    }
}

impl CaptureFailure {
    pub(crate) fn response(&self) -> serde_json::Value {
        let mut fields = serde_json::Map::new();
        fields.insert("error".into(), self.code().into());
        fields.insert("message".into(), self.to_string().into());
        if let Self::Native(error) = self {
            for (name, value) in [
                ("packageKey", &error.package_key),
                ("instanceId", &error.instance_id),
                ("requestId", &error.request_id),
            ] {
                if let Some(value) = value {
                    fields.insert(name.into(), value.clone().into());
                }
            }
            if let Some(value) = &error.data_error {
                fields.insert("data".into(), serde_json::json!(value));
            }
            if let Some(value) = &error.save_outcome {
                fields.insert("saveOutcome".into(), serde_json::json!(value));
            }
            if let Some(value) = &error.save_database_error {
                fields.insert("database".into(), serde_json::json!(value));
            }
            if let Some(value) = &error.save_rollback_error {
                fields.insert("rollback".into(), serde_json::json!(value));
            }
            if let Some(value) = error.exit_code {
                fields.insert("exitCode".into(), value.into());
            }
        }
        serde_json::Value::Object(fields)
    }
}

impl CaptureFailure {
    pub(crate) fn into_native(self) -> Box<NativeHostError> {
        if let Self::Native(error) = self {
            return error;
        }
        if let Self::Unknown { cause, rollback } = self {
            let mut error = cause.into_native();
            error.save_outcome =
                Some(crate::runtime_plugins::literature_catalog::save::SaveOutcome::Unknown);
            error.message = format!(
                "采集结果尚未确认，需按存储恢复流程核对原会话回执。{}",
                error.message
            );
            if let Some(cause) = &rollback {
                error.message.push_str(&format!("；rollback={cause}"));
            }
            error.save_rollback_error = rollback;
            return error;
        }
        let mut error = NativeHostError::for_profile(
            crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
            self.code(),
            self.to_string(),
        );
        match self {
            Self::Data(cause) => error.data_error = Some(cause),
            Self::Database(cause) => error.save_database_error = Some(cause),
            _ => {}
        }
        error
    }
}
