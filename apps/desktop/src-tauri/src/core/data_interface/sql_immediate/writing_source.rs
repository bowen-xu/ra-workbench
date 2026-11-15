//! Bridge request-local source pauses through the original transaction shell.
use super::*;
use crate::{
    agent::writing_context::ContextNeed,
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
};

pub(super) type SourceResult<T, P = ContextNeed> = Result<T, Flow<SqlImmediateFailure, P>>;
pub(super) struct SourcePause<P = ContextNeed> {
    operation: Uuid,
    phase: &'static str,
    need: Option<P>,
    marker: &'static str,
    cause: FailureCause,
}
impl SourcePause<ContextNeed> {
    pub fn new(operation: Uuid, phase: &'static str) -> Self {
        Self {
            operation,
            phase,
            need: None,
            marker: "writing_source_decision_required",
            cause: FailureCause::Original,
        }
    }
}
impl<P> SourcePause<P> {
    pub fn capture<T>(
        &mut self,
        result: SourceResult<T, P>,
        fence: ReadFence,
    ) -> Result<T, SqlImmediateFailure> {
        match result {
            Ok(value) => Ok(value),
            Err(Flow::Fail(failure)) => {
                self.cause = failure.cause;
                Err(failure.error)
            }
            Err(Flow::Need(input)) => {
                self.need = Some(input);
                Err(SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                    self.operation,
                    fence,
                    self.phase,
                    self.marker,
                ))))
            }
        }
    }
    /// execute_sql confirms rollback before returning the private pause marker.
    /// Its Unknown cleanup result takes priority over any pending Native input.
    pub fn finish<T>(self, result: Result<T, SqlImmediateFailure>) -> SourceResult<T, P> {
        match result {
            Ok(value) => Ok(value),
            Err(SqlImmediateFailure::Unavailable(detail))
                if detail.phase == self.phase && detail.safe_cause == self.marker =>
            {
                Err(Flow::Need(
                    self.need.expect("source pause requires an exact input"),
                ))
            }
            Err(error) => Err(Flow::Fail(Failure {
                error,
                cause: self.cause,
            })),
        }
    }
}

impl SourcePause<crate::agent::reading_context::ReadingNeed> {
    pub fn paper(operation: Uuid, phase: &'static str) -> Self {
        Self {
            operation,
            phase,
            need: None,
            cause: FailureCause::Original,
            marker: "paper_source_decision_required",
        }
    }
}
