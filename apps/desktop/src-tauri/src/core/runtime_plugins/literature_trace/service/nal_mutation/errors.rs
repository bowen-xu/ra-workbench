use super::*;
use crate::{
    data_interface::DataError,
    runtime_plugins::{
        literature_nal_bootstrap::{BootstrapFailure, BootstrapOutcome},
        literature_nal_mutation::{MutationCause, MutationFailure, MutationPhase, MutationProblem},
        native::SafeDatabaseError,
    },
};

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
            MutationCause::Database(Box::new(SafeDatabaseError::sql("owner", &error)))
        }
        _ => MutationCause::Owner(kind),
    };
    let mut error =
        LiteratureTraceService::error("storage_error", format!("关系存储暂时不可用：{kind}"));
    error.mutation_problem = Some(Box::new(MutationProblem {
        phase: MutationPhase::Startup,
        cause,
    }));
    error
}

pub(super) fn codec() -> Box<NativeHostError> {
    let mut error =
        LiteratureTraceService::error("plugin_host_input_invalid", "关系修改输入无法编码。");
    error.mutation_problem = Some(Box::new(MutationProblem::binding(
        MutationPhase::Mutation,
        "request-codec",
    )));
    error
}

pub(super) fn capture(failure: BootstrapFailure) -> Box<NativeHostError> {
    let outcome = match failure.outcome {
        BootstrapOutcome::NotStarted | BootstrapOutcome::RolledBack => MutationOutcome::NotStarted,
        BootstrapOutcome::Unknown { .. } | BootstrapOutcome::Committed { .. } => {
            MutationOutcome::Unknown {
                observed_revision: None,
            }
        }
    };
    self::failure(MutationFailure {
        problem: MutationProblem {
            phase: MutationPhase::Startup,
            cause: MutationCause::Startup(Box::new(failure.problem)),
        },
        outcome,
        rollback_error: failure.rollback_error,
    })
}

pub(super) fn failure(failure: MutationFailure) -> Box<NativeHostError> {
    let code = if matches!(failure.outcome, MutationOutcome::Unknown { .. }) {
        "storage_error"
    } else {
        failure.problem.code()
    };
    let mut error = LiteratureTraceService::error(code, failure.to_string());
    error.mutation_problem = Some(Box::new(failure.problem));
    error.mutation_outcome = Some(failure.outcome);
    error.mutation_rollback_error = failure.rollback_error;
    error
}

pub(super) fn unknown(message: &str) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error("storage_error", message);
    error.mutation_outcome = Some(MutationOutcome::Unknown {
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
