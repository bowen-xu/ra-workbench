//! Typed adaptation of the existing shared transaction settlement; no SQL owner.
use super::super::Error;
use crate::{
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
    hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome, Settlement},
    runtime_plugins::native::SafeDatabaseError,
};
use rusqlite::{Connection, Transaction};

pub(in crate::structured_resources) fn before<P>(
    error: impl Into<Error>,
    phase: &'static str,
) -> BoundaryError<P, Error> {
    let error = error.into();
    let cause = match &error {
        Error::Database(error) => {
            FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(phase, error)))
        }
        _ => FailureCause::Original,
    };
    BoundaryError::Failure(BoundaryFailure {
        failure: Failure { error, cause },
        phase,
        outcome: HbsOutcome::NotStarted,
        rollback_error: None,
    })
}
pub(in crate::structured_resources) fn settle<P, T>(
    tx: Transaction<'_>,
    result: Result<(T, bool), Flow<Error, P>>,
    phase: &'static str,
    explicit: bool,
) -> Settlement<P, T, Error> {
    let result = result.map_err(|error| match error {
        Flow::Fail(mut failure) => {
            if matches!(failure.cause, FailureCause::Original)
                && let Error::Database(error) = &failure.error
            {
                failure.cause =
                    FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(phase, error)));
            }
            Flow::Fail(failure)
        }
        error => error,
    });
    let (result, commit) = match result {
        Ok((value, commit)) => (Ok(value), commit),
        Err(error) => (Err(error), false),
    };
    boundary::settle_with(
        tx,
        result,
        phase,
        explicit,
        commit,
        Error::Database,
        |message| Error::Invalid(message.into()),
    )
}
pub(in crate::structured_resources) fn confirm<P, T>(
    db: &Connection,
    settled: Settlement<P, T, Error>,
    explicit: bool,
) -> Result<T, BoundaryError<P, Error>> {
    boundary::confirm_with(db, settled, explicit, |message| {
        Error::Invalid(message.into())
    })
}
