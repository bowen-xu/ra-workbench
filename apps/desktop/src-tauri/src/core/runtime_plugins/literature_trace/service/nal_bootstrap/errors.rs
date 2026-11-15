use super::*;
use crate::{
    data_interface::DataError,
    runtime_plugins::{
        literature_nal_bootstrap::{
            BootstrapCause, BootstrapFailure, BootstrapProblem, FailureContext,
        },
        native::SafeDatabaseError,
    },
};
use research_workbench_native_sdk::nal_bootstrap::BootstrapPhaseV1;

pub(super) fn owner(cause: DataError) -> Box<NativeHostError> {
    let kind = match &cause {
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Sql(_) => "DATA_RUNTIME_STORAGE",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    let cause = match cause {
        DataError::Sql(error) => {
            BootstrapCause::Database(Box::new(SafeDatabaseError::sql("owner", &error)))
        }
        _ => BootstrapCause::Owner(kind),
    };
    let mut error =
        LiteratureTraceService::error("storage_error", format!("关系存储暂时不可用：{kind}"));
    error.bootstrap_problem = Some(Box::new(BootstrapProblem {
        phase: BootstrapPhaseV1::LegacyHeader,
        context: FailureContext::Network,
        cause,
    }));
    error
}

pub(super) fn failure(failure: BootstrapFailure) -> Box<NativeHostError> {
    let code = if matches!(failure.outcome, BootstrapOutcome::Unknown { .. }) {
        "storage_error"
    } else {
        failure.problem.code()
    };
    let mut error = LiteratureTraceService::error(code, failure.to_string());
    error.bootstrap_problem = Some(Box::new(failure.problem));
    error.bootstrap_outcome = Some(failure.outcome);
    error.bootstrap_rollback_error = failure.rollback_error;
    error
}

pub(super) fn unknown(message: &str) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error("storage_error", message);
    error.bootstrap_outcome = Some(BootstrapOutcome::Unknown {
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
