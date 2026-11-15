//! Shared original Deferred settlement for bibliography adoption and rekey.
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
    runtime_plugins::native::SafeDatabaseError,
};
use rusqlite::{Connection, DropBehavior, Transaction};
#[derive(Clone, Copy)]
pub(super) struct Phases {
    pub(super) begin: &'static str,
    pub(super) commit: &'static str,
    pub(super) rollback: &'static str,
    pub(super) unsettled: &'static str,
}
pub(crate) struct Attempt<T, Need> {
    pub(crate) result: Result<T, Flow<WritingError, Need>>,
    pub(crate) requires_recovery: bool,
    pub(crate) rollback_error: Option<SafeDatabaseError>,
}
pub(super) fn sql_failure<Need>(
    cause: rusqlite::Error,
    phase: &'static str,
) -> Flow<WritingError, Need> {
    let diagnostic = SafeDatabaseError::sql(phase, &cause);
    Flow::Fail(Failure {
        error: WritingError::Database(cause),
        cause: FailureCause::SourceDatabase(Box::new(diagnostic)),
    })
}
pub(super) fn run<T, Need>(
    db: &Connection,
    phases: Phases,
    body: impl FnOnce(&Transaction<'_>) -> Result<T, Flow<WritingError, Need>>,
) -> Attempt<T, Need> {
    settle(db, phases, Finish::Commit, body)
}
pub(super) fn read<T, Need>(
    db: &Connection,
    phases: Phases,
    body: impl FnOnce(&Transaction<'_>) -> Result<T, Flow<WritingError, Need>>,
) -> Attempt<T, Need> {
    settle(db, phases, Finish::Rollback, body)
}
enum Finish {
    Commit,
    Rollback,
}
fn settle<T, Need>(
    db: &Connection,
    phases: Phases,
    finish: Finish,
    body: impl FnOnce(&Transaction<'_>) -> Result<T, Flow<WritingError, Need>>,
) -> Attempt<T, Need> {
    let mut tx = match db.unchecked_transaction() {
        Ok(tx) => tx,
        Err(cause) => {
            return Attempt {
                result: Err(sql_failure(cause, phases.begin)),
                requires_recovery: !db.is_autocommit(),
                rollback_error: None,
            };
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let result = body(&tx);
    if result.is_ok() && matches!(finish, Finish::Commit) {
        match tx.commit() {
            Ok(()) if db.is_autocommit() => {
                return Attempt {
                    result,
                    requires_recovery: false,
                    rollback_error: None,
                };
            }
            Ok(()) => {
                let diagnostic = SafeDatabaseError::invariant(phases.unsettled, phases.commit);
                return Attempt {
                    result: Err(unsettled(diagnostic.clone())),
                    requires_recovery: true,
                    rollback_error: Some(diagnostic),
                };
            }
            Err(cause) => {
                let result = Err(sql_failure(cause, phases.commit));
                if db.is_autocommit() {
                    return Attempt {
                        result,

                        requires_recovery: true,
                        rollback_error: None,
                    };
                }
                return rolled_back(db, phases, result, db.execute_batch("ROLLBACK"));
            }
        }
    }
    rolled_back(db, phases, result, tx.rollback())
}

fn rolled_back<T, Need>(
    db: &Connection,
    phases: Phases,
    mut result: Result<T, Flow<WritingError, Need>>,
    rollback: rusqlite::Result<()>,
) -> Attempt<T, Need> {
    let diagnostic = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Ok(()) => Some(SafeDatabaseError::invariant(
            phases.unsettled,
            phases.rollback,
        )),
        Err(cause) => {
            let diagnostic = SafeDatabaseError::sql(phases.rollback, &cause);
            if result.is_ok() || matches!(&result, Err(Flow::Need(_))) {
                result = Err(sql_failure(cause, phases.rollback));
            }
            Some(diagnostic)
        }
    };
    if let Some(diagnostic) = &diagnostic
        && (result.is_ok() || matches!(&result, Err(Flow::Need(_))))
    {
        result = Err(unsettled(diagnostic.clone()));
    }
    Attempt {
        result,

        requires_recovery: diagnostic.is_some(),
        rollback_error: diagnostic,
    }
}

fn unsettled<Need>(diagnostic: SafeDatabaseError) -> Flow<WritingError, Need> {
    Flow::Fail(Failure {
        error: WritingError::Storage(crate::storage::StorageError::DataInterface(
            crate::data_interface::DataError::Unavailable,
        )),
        cause: FailureCause::SourceDatabase(Box::new(diagnostic)),
    })
}
