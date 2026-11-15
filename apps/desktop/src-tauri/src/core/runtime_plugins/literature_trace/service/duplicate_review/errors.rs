use super::*;
use crate::{
    data_interface::{DataError, plugin_resources::PluginDataError},
    runtime_plugins::{literature_duplicates::DecisionSqlState, native::SafeDatabaseError},
};

pub(super) fn data(cause: Box<PluginDataError>) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error(
        "storage_error",
        format!("{}：{}", cause.message, cause.safe_cause),
    );
    error.data_error = Some(cause);
    error
}

pub(super) fn owner(cause: DataError) -> Box<NativeHostError> {
    let kind = match &cause {
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Sql(_) => "DATA_RUNTIME_STORAGE",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    let mut error =
        LiteratureTraceService::error("storage_error", format!("重复项存储暂时不可用：{kind}"));
    if let DataError::Sql(cause) = cause {
        error.duplicate_database_error = Some(Box::new(SafeDatabaseError::sql("owner", &cause)));
    }
    error
}

pub(super) fn unknown(message: &str, writes: bool) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error("storage_error", message);
    if writes {
        error.duplicate_sql = Some(DecisionSqlState::Unknown);
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
pub(super) fn scope_request(
    mut error: Box<NativeHostError>,
    before: Option<&(String, String)>,
) -> Box<NativeHostError> {
    if let Some((instance, request)) = before
        && error.instance_id.as_ref() == Some(instance)
        && error.request_id.as_ref() == Some(request)
    {
        error.request_id = None;
    }
    error
}
