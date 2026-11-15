//! Current-file reads reuse the existing Native preparation reader and settlement.
use super::*;
use crate::{
    agent::writing_context::{ContextNeed, WritingContextError, WritingContextReader},
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
    data_interface::guarded_transaction,
};
use rusqlite::Connection;

#[derive(Clone, Copy)]
pub(crate) enum ContextReadMode {
    Snapshot,
    CoreProofs,
}

impl Database {
    pub(crate) fn read_writing_context<T>(
        &self,
        operation: Uuid,
        mode: ContextReadMode,
        reader: &mut dyn WritingContextReader,
        read: impl FnOnce(
            &mut Connection,
            &mut dyn WritingContextReader,
        ) -> Result<T, Flow<WritingContextError, ContextNeed>>,
    ) -> Result<T, Flow<SqlImmediateFailure, ContextNeed>> {
        let diagnostic =
            |phase, cause| SqlImmediateDiagnostic::for_operation(operation, None, phase, cause);
        let mut guard = self.lock().map_err(|_| {
            Flow::original(SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                "writing-source-read",
                "source_owner_unavailable",
            ))))
        })?;
        guarded_transaction::run_mut(&mut guard, |db| {
            if !db.is_autocommit() {
                return (
                    Err(Flow::original(SqlImmediateFailure::Unknown(Box::new(
                        diagnostic("writing-source-read", "source_owner_unsettled"),
                    )))),
                    true,
                );
            }
            if matches!(mode, ContextReadMode::CoreProofs) {
                // Core Note reads retain their original autocommit read proofs.
                // Replays use the same idempotent proof identity. Only a settled
                // prefix may release this guard and wait for the writing host.
                let result = read(db, reader).map_err(|flow| read_error(flow, operation));
                if !db.is_autocommit() {
                    let cause = match result {
                        Err(Flow::Fail(failure)) => failure.cause,
                        _ => FailureCause::Original,
                    };
                    return (
                        Err(Flow::Fail(Failure {
                            error: SqlImmediateFailure::Unknown(Box::new(diagnostic(
                                "context-source-read",
                                "core_read_proof_owner_unsettled",
                            ))),
                            cause,
                        })),
                        true,
                    );
                }
                return (result, false);
            }
            // The ordered resource evaluator borrows a mutable Connection for its
            // existing callbacks. Keep this read-only consumer in one explicit
            // Deferred transaction and fence any unconfirmed settlement.
            if let Err(error) = db.execute_batch("begin deferred") {
                let unknown = !db.is_autocommit();
                let detail = Box::new(
                    diagnostic("writing-source-read", "source_read_begin_failed").sqlite(&error),
                );
                let failure = if unknown {
                    SqlImmediateFailure::Unknown(detail)
                } else {
                    SqlImmediateFailure::Unavailable(detail)
                };
                return (Err(Flow::original(failure)), unknown);
            }
            let result = read(db, reader).map_err(|flow| read_error(flow, operation));
            let rollback = db.execute_batch("rollback");
            let unknown = rollback.is_err() || !db.is_autocommit();
            if unknown {
                let cause = match result {
                    Err(Flow::Fail(failure)) => failure.cause,
                    _ => FailureCause::Original,
                };
                let mut detail = diagnostic(
                    "writing-source-read-rollback",
                    "source_read_rollback_unconfirmed",
                );
                if let Err(error) = rollback {
                    detail = detail.sqlite(&error);
                }
                return (
                    Err(Flow::Fail(Failure {
                        error: SqlImmediateFailure::Unknown(Box::new(detail)),
                        cause,
                    })),
                    true,
                );
            }
            (result, false)
        })
    }
}

fn read_error(
    flow: Flow<WritingContextError, ContextNeed>,
    operation: Uuid,
) -> Flow<SqlImmediateFailure, ContextNeed> {
    flow.map_error(|error| {
        let cause = match error {
            WritingContextError::InvalidInput => "writing_context_invalid_input",
            WritingContextError::Conflict => "writing_context_changed",
            WritingContextError::Unavailable => "writing_context_unavailable",
            WritingContextError::Budget => "writing_context_budget",
        };
        SqlImmediateFailure::Unavailable(Box::new(SqlImmediateDiagnostic::for_operation(
            operation,
            None,
            "writing-source-read",
            cause,
        )))
    })
}

#[cfg(test)]
#[path = "writing_context_read_tests.rs"]
mod tests;
