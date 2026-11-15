use super::*;
use crate::{
    data_interface::DataError,
    runtime_plugins::{
        literature_duplicate_merge::{MergeCause, MergeFailure, MergeProblem},
        native::SafeDatabaseError,
    },
};

pub(super) fn failure(failure: MergeFailure) -> Box<NativeHostError> {
    let code = if matches!(failure.outcome, MergeOutcome::Unknown) {
        "storage_error"
    } else {
        failure.problem.code()
    };
    let mut error = LiteratureTraceService::error(code, failure.to_string());
    error.operation_id = failure.operation_id.clone();
    error.duplicate_merge = Some(Box::new(failure));
    error
}

pub(super) fn problem(problem: MergeProblem, outcome: MergeOutcome) -> Box<NativeHostError> {
    failure(MergeFailure {
        problem,
        outcome,
        rollback_error: None,
        operation_id: None,
    })
}

pub(super) fn owner(phase: MergePhase, cause: DataError) -> Box<NativeHostError> {
    let kind = match &cause {
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Sql(_) => "DATA_RUNTIME_STORAGE",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    let cause = match cause {
        DataError::Sql(error) => {
            MergeCause::Database(Box::new(SafeDatabaseError::sql("owner", &error)))
        }
        _ => MergeCause::Owner(kind),
    };
    problem(MergeProblem { phase, cause }, MergeOutcome::NotStarted)
}

pub(super) fn unknown(message: &str) -> Box<NativeHostError> {
    let mut error = problem(
        MergeProblem {
            phase: MergePhase::Unknown,
            cause: MergeCause::Owner("MERGE_WORKER_UNSETTLED"),
        },
        MergeOutcome::Unknown,
    );
    error.message = message.into();
    error
}

pub(super) fn capture_failed() -> Box<NativeHostError> {
    problem(
        MergeProblem {
            phase: MergePhase::Header,
            cause: MergeCause::Owner("MERGE_CAPTURE_WORKER_FAILED"),
        },
        MergeOutcome::NotStarted,
    )
}

pub(super) fn set_not_started(error: &mut NativeHostError) {
    error.duplicate_merge = Some(Box::new(MergeFailure {
        problem: MergeProblem {
            phase: MergePhase::Preparation,
            cause: MergeCause::Owner("MERGE_NOT_DISPATCHED"),
        },
        outcome: MergeOutcome::NotStarted,
        rollback_error: None,
        operation_id: None,
    }));
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
