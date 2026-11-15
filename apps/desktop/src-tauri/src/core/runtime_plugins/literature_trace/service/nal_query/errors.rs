use super::*;
use crate::runtime_plugins::{
    literature_nal_query::{QueryCause, QueryFailure},
    native::SafeDatabaseError,
};

pub(super) fn owner(cause: crate::data_interface::DataError) -> Box<NativeHostError> {
    use crate::data_interface::DataError;
    let kind = match &cause {
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Sql(_) => "DATA_RUNTIME_STORAGE",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    let mut error =
        LiteratureTraceService::error("storage_error", format!("查询集合暂时不可用：{kind}"));
    error.query_error_kind = Some(kind);
    if let DataError::Sql(sql) = cause {
        error.query_database_error = Some(Box::new(SafeDatabaseError::sql("owner", &sql)));
    }
    error
}

pub(super) fn cause(cause: QueryCause) -> Box<NativeHostError> {
    let code = match &cause {
        QueryCause::Conflict | QueryCause::InvalidStructure => "invalid_input",
        _ => "storage_error",
    };
    let mut error = LiteratureTraceService::error(code, cause.to_string());
    attach(&mut error, cause);
    error
}

pub(super) fn failure(failure: QueryFailure) -> Box<NativeHostError> {
    let code = match (&failure.cause, &failure.outcome) {
        (QueryCause::Conflict | QueryCause::InvalidStructure, QueryOutcome::RolledBack) => {
            "invalid_input"
        }
        _ => "storage_error",
    };
    // QueryFailure exposes the original CAS retry phrase only after a confirmed
    // rollback. Do not append its primary cause to an unknown public message.
    let mut error = LiteratureTraceService::error(code, failure.to_string());
    error.query_outcome = Some(failure.outcome);
    error.query_rollback_error = failure.rollback_error;
    attach(&mut error, failure.cause);
    error
}

fn attach(error: &mut NativeHostError, cause: QueryCause) {
    match cause {
        QueryCause::Conflict => error.query_error_kind = Some("nal_query_cas_conflict"),
        QueryCause::InvalidStructure => {
            error.query_error_kind = Some("nal_query_invalid_structure")
        }
        QueryCause::Data(cause) => error.data_error = Some(cause),
        QueryCause::Database(cause) => error.query_database_error = Some(cause),
    }
}

pub(super) fn unknown(message: &str) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error("storage_error", message);
    error.query_outcome = Some(QueryOutcome::Unknown {
        observed_revision: None,
    });
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

pub(super) fn rejected(reason: Rejection) -> Box<NativeHostError> {
    let message = match reason {
        Rejection::TooLarge => "查询缓存过大",
        Rejection::InvalidDefinition => "查询集合格式无效",
        Rejection::InvalidHierarchy => "查询目录存在循环或超过 16 个条件",
        Rejection::MissingParent => "上级查询已不存在",
        Rejection::InvalidOutputVariables => "输出变量无效，请从语句中选择",
        Rejection::InvalidStructure => "关系数据结构无效",
    };
    LiteratureTraceService::error("invalid_input", message)
}
