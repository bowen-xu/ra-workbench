use super::*;
use crate::{
    data_interface::DataError,
    runtime_plugins::literature_duplicate_undo::{UndoCause, UndoFailure, UndoPhase, UndoProblem},
};

pub(super) fn failure(failure: UndoFailure) -> Box<NativeHostError> {
    let code = if failure.outcome == UndoOutcome::Unknown {
        "storage_error"
    } else {
        failure.problem.code()
    };
    let mut error = LiteratureTraceService::error(code, failure.to_string());
    error.duplicate_undo = Some(Box::new(failure));
    error
}
pub(super) fn problem(problem: UndoProblem, outcome: UndoOutcome) -> Box<NativeHostError> {
    failure(UndoFailure {
        problem,
        outcome,
        rollback_error: None,
    })
}
pub(super) fn owner(cause: DataError) -> Box<NativeHostError> {
    let kind = match &cause {
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Sql(_) => "DATA_RUNTIME_STORAGE",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    let cause = match cause {
        DataError::Sql(error) => UndoCause::Database(Box::new(
            crate::runtime_plugins::native::SafeDatabaseError::sql("owner", &error),
        )),
        _ => UndoCause::Owner(kind),
    };
    problem(
        UndoProblem {
            phase: UndoPhase::Header,
            cause,
        },
        UndoOutcome::NotStarted,
    )
}
pub(super) fn unknown(message: &str) -> Box<NativeHostError> {
    let mut error = problem(
        UndoProblem {
            phase: UndoPhase::Unknown,
            cause: UndoCause::Owner("UNDO_WORKER_UNSETTLED"),
        },
        UndoOutcome::Unknown,
    );
    error.message = message.into();
    error
}
pub(super) fn set_not_started(error: &mut NativeHostError) {
    error.duplicate_undo = Some(Box::new(UndoFailure {
        problem: UndoProblem {
            phase: UndoPhase::Preparation,
            cause: UndoCause::Owner("UNDO_NOT_DISPATCHED"),
        },
        outcome: UndoOutcome::NotStarted,
        rollback_error: None,
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
