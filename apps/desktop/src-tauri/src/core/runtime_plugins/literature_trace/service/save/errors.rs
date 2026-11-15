use super::*;
use crate::runtime_plugins::literature_catalog::save::{SaveCause, SaveFailure};
use research_workbench_native_sdk::catalog_save::CatalogSaveRejection;

pub(super) fn owner_failure(cause: crate::data_interface::DataError) -> Box<NativeHostError> {
    use crate::data_interface::DataError;
    let category = match &cause {
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Sql(_) => "DATA_RUNTIME_STORAGE",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    let mut error =
        LiteratureTraceService::error("storage_error", format!("Core数据拥有者不可用：{category}"));
    if let DataError::Sql(sql) = cause {
        let sqlite = match &sql {
            rusqlite::Error::SqliteFailure(code, _)
            | rusqlite::Error::SqlInputError { error: code, .. } => Some(code),
            _ => None,
        };
        error.save_database_error = Some(Box::new(save::SafeDatabaseError {
            code: "storage_error",
            phase: "owner",
            safe_cause: "owner_sql_failed",
            sqlite_code: sqlite.map(|code| format!("{:?}", code.code)),
            sqlite_extended_code: sqlite.map(|code| code.extended_code),
            original_sql_failure: None,
        }));
    }
    error
}

pub(super) fn unknown(message: &str) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error("storage_error", message);
    error.save_outcome = Some(SaveOutcome::Unknown);
    error
}

pub(super) fn storage(failure: SaveFailure) -> Box<NativeHostError> {
    let mut error =
        LiteratureTraceService::error("storage_error", format!("文献数据库操作失败：{failure}"));
    error.save_outcome = Some(failure.outcome);
    error.save_rollback_error = failure.rollback_error;
    match failure.cause {
        SaveCause::Data(cause) => error.data_error = Some(cause),
        SaveCause::Database(cause) => error.save_database_error = Some(cause),
    }
    error
}

#[cfg(unix)]
pub(super) fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}

#[cfg(unix)]
pub(super) fn rejected(host: &NativeHost, reason: CatalogSaveRejection) -> Box<NativeHostError> {
    let message = match reason {
        CatalogSaveRejection::UnsupportedType { item_type } => {
            format!("不支持的文献类型：{item_type}")
        }
        CatalogSaveRejection::EmptyTitle => "标题不能为空".into(),
    };
    host.error("invalid_input", message)
}
