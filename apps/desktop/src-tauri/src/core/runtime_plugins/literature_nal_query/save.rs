use super::{NalQueryResources, QueryCause, QueryFailure, QueryOutcome};
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction},
    runtime_plugins::native::SafeDatabaseError,
};
use research_workbench_native_sdk::nal_query::{SavePlan, Stored};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

/// The Native semantic decision precedes this call. Only the original CAS,
/// mechanical SDK JSON decode and deferred transaction execute under this guard.
pub(crate) fn save(
    db: &mut DatabaseGuard<'_>,
    resources: &NalQueryResources,
    plan: &SavePlan,
    expected_revision: i64,
) -> Result<Stored, QueryFailure> {
    guarded_transaction::run(db, |connection| {
        run(connection, resources, plan, expected_revision)
    })
}

fn run(
    connection: &Connection,
    resources: &NalQueryResources,
    plan: &SavePlan,
    expected_revision: i64,
) -> (Result<Stored, QueryFailure>, bool) {
    let was_autocommit = connection.is_autocommit();
    let mut tx = match Transaction::new_unchecked(connection, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(error) => {
            let unknown = was_autocommit && !connection.is_autocommit();
            return (
                Err(QueryFailure {
                    cause: QueryCause::Database(Box::new(SafeDatabaseError::sql("begin", &error))),
                    outcome: if unknown {
                        QueryOutcome::Unknown {
                            observed_revision: None,
                        }
                    } else {
                        QueryOutcome::NotStarted
                    },
                    rollback_error: None,
                }),
                unknown,
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let mut observed_revision = None;
    let result = (|| {
        if resources.update_in(&tx, plan, expected_revision)? != 1 {
            return Err(QueryCause::Conflict);
        }
        let row = resources.read_raw(&tx)?;
        observed_revision = Some(row.revision);
        row.decode().map_err(|_| QueryCause::InvalidStructure)
    })();
    let stored = match result {
        Ok(stored) => stored,
        Err(cause) => return failed(connection, cause, tx.rollback(), observed_revision),
    };
    if let Err(error) = tx.commit() {
        let cause = QueryCause::Database(Box::new(SafeDatabaseError::sql("commit", &error)));
        if connection.is_autocommit() {
            return (
                Err(QueryFailure {
                    cause,
                    outcome: QueryOutcome::Unknown { observed_revision },
                    rollback_error: None,
                }),
                true,
            );
        }
        return failed(
            connection,
            cause,
            connection.execute_batch("ROLLBACK"),
            observed_revision,
        );
    }
    if !connection.is_autocommit() {
        return (
            Err(QueryFailure {
                cause: QueryCause::Database(Box::new(SafeDatabaseError::invariant(
                    "nal_query_transaction_unsettled",
                    "commit",
                ))),
                outcome: QueryOutcome::Unknown { observed_revision },
                rollback_error: None,
            }),
            true,
        );
    }
    (Ok(stored), false)
}

fn failed<T>(
    connection: &Connection,
    cause: QueryCause,
    rollback: rusqlite::Result<()>,
    observed_revision: Option<i64>,
) -> (Result<T, QueryFailure>, bool) {
    let rollback_error = match rollback {
        Ok(()) if connection.is_autocommit() => None,
        Err(error) => Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "nal_query_transaction_unsettled",
            "rollback",
        ))),
    };
    let unknown = rollback_error.is_some();
    (
        Err(QueryFailure {
            cause,
            outcome: if unknown {
                QueryOutcome::Unknown { observed_revision }
            } else {
                QueryOutcome::RolledBack
            },
            rollback_error,
        }),
        unknown,
    )
}
