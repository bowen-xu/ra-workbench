use super::*;
use crate::{
    ai::structured_resource_validation::{Failure, FailureCause},
    runtime_plugins::native::SafeDatabaseError,
};
pub(super) fn original(error: AiError) -> ContextFailure {
    Box::new(BoundaryFailure {
        failure: Failure::original(error),
        phase: "source-read",
        outcome: HbsOutcome::NotStarted,
        rollback_error: None,
    })
}
pub(crate) fn sql(error: rusqlite::Error, phase: &'static str) -> ContextFailure {
    let cause = FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(phase, &error)));
    Box::new(BoundaryFailure {
        failure: Failure {
            error: AiError::Database(error),
            cause,
        },
        phase,
        outcome: HbsOutcome::NotStarted,
        rollback_error: None,
    })
}
pub(crate) fn ai(error: AiError) -> ContextFailure {
    match error {
        AiError::Database(error) => sql(error, "source-read"),
        error => original(error),
    }
}
pub(super) fn data(error: Box<PluginDataError>) -> ContextFailure {
    Box::new(BoundaryFailure {
        failure: Failure {
            error: AiError::TemporaryUnavailable,
            cause: FailureCause::SourceData(error),
        },
        phase: "source-read",
        outcome: HbsOutcome::NotStarted,
        rollback_error: None,
    })
}
pub(super) fn invalid(message: &str) -> ContextFailure {
    original(AiError::InvalidInput(message.into()))
}
pub(super) fn changed() -> ContextFailure {
    invalid("论文上下文计算所依据的数据已变化，请重试原请求。")
}
