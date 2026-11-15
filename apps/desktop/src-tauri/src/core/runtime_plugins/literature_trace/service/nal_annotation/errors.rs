use super::*;
use crate::{
    data_interface::DataError,
    runtime_plugins::{
        literature_nal_annotation::{
            AnnotationCause, AnnotationFailure, AnnotationPhase, AnnotationProblem,
            AnnotationRejection,
        },
        literature_nal_bootstrap::{BootstrapFailure, BootstrapOutcome},
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
            AnnotationCause::Database(Box::new(SafeDatabaseError::sql("owner", &error)))
        }
        _ => AnnotationCause::Owner(kind),
    };
    let mut error =
        LiteratureTraceService::error("storage_error", format!("笔记存储暂时不可用：{kind}"));
    error.annotation_problem = Some(Box::new(AnnotationProblem {
        phase: AnnotationPhase::Startup,
        cause,
    }));
    error
}

pub(super) fn invalid_request() -> Box<NativeHostError> {
    failure(AnnotationFailure {
        problem: AnnotationProblem::rejected(
            AnnotationPhase::Validation,
            AnnotationRejection::InvalidRequest,
        ),
        outcome: AnnotationOutcome::NotStarted,
        rollback_error: None,
    })
}

pub(super) fn capture(failure: BootstrapFailure) -> Box<NativeHostError> {
    let outcome = match failure.outcome {
        BootstrapOutcome::NotStarted | BootstrapOutcome::RolledBack => {
            AnnotationOutcome::NotStarted
        }
        BootstrapOutcome::Unknown { .. } | BootstrapOutcome::Committed { .. } => {
            AnnotationOutcome::Unknown
        }
    };
    self::failure(AnnotationFailure {
        problem: AnnotationProblem {
            phase: AnnotationPhase::Startup,
            cause: AnnotationCause::Startup(Box::new(failure.problem)),
        },
        outcome,
        rollback_error: failure.rollback_error,
    })
}

pub(super) fn failure(failure: AnnotationFailure) -> Box<NativeHostError> {
    let code = if matches!(failure.outcome, AnnotationOutcome::Unknown) {
        "storage_error"
    } else {
        failure.problem.code()
    };
    let mut error = LiteratureTraceService::error(code, failure.to_string());
    error.annotation_problem = Some(Box::new(failure.problem));
    error.annotation_outcome = Some(failure.outcome);
    error.annotation_rollback_error = failure.rollback_error;
    error
}

pub(super) fn unknown(message: &str) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error("storage_error", message);
    error.annotation_outcome = Some(AnnotationOutcome::Unknown);
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
