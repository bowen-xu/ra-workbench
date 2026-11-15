use super::{
    BootstrapFailure, BootstrapOutcome, BootstrapProblem, BootstrapResources, FailureContext,
    PreparedBootstrapFacts, PreparedBootstrapSource, outcome::BootstrapCause,
};
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction},
    runtime_plugins::native::SafeDatabaseError,
};
use research_workbench_native_sdk::nal_bootstrap::{
    BootstrapIdentitiesV1, BootstrapPhaseV1, BootstrapProbePreparationV1,
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

/// Complete one short R0 read transaction before any native wait. Its result
/// owns all observations and contains no borrow of the connection or guard.
pub(crate) fn capture_source(
    guard: &mut DatabaseGuard<'_>,
    resources: &BootstrapResources,
) -> Result<PreparedBootstrapSource, BootstrapFailure> {
    guarded_transaction::run(guard, |db| {
        let was_autocommit = db.is_autocommit();
        let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
            Ok(tx) => tx,
            Err(error) => {
                let unknown = was_autocommit && !db.is_autocommit();
                return (
                    Err(BootstrapFailure {
                        problem: database_problem("begin", &error),
                        outcome: if unknown {
                            BootstrapOutcome::Unknown {
                                observed_revision: None,
                            }
                        } else {
                            BootstrapOutcome::NotStarted
                        },
                        rollback_error: None,
                    }),
                    unknown,
                );
            }
        };
        // A panic leaves settlement to the existing guarded_transaction fence,
        // rather than silently treating Drop's best-effort rollback as a receipt.
        tx.set_drop_behavior(DropBehavior::Ignore);
        let source = resources.capture_source(&tx);
        settle(db, Ok(source), tx.rollback())
    })
}

/// Recapture R0 and derive its probes inside this one short R1 transaction;
/// rollback is confirmed before returning owned facts to the second native call.
pub(crate) fn capture_facts(
    guard: &mut DatabaseGuard<'_>,
    resources: &BootstrapResources,
    previous: &PreparedBootstrapSource,
    probes: &BootstrapProbePreparationV1,
    identities: &BootstrapIdentitiesV1,
) -> Result<PreparedBootstrapFacts, BootstrapFailure> {
    guarded_transaction::run(guard, |db| {
        let was_autocommit = db.is_autocommit();
        let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
            Ok(tx) => tx,
            Err(error) => {
                let unknown = was_autocommit && !db.is_autocommit();
                return (
                    Err(BootstrapFailure {
                        problem: database_problem("begin", &error),
                        outcome: if unknown {
                            BootstrapOutcome::Unknown {
                                observed_revision: None,
                            }
                        } else {
                            BootstrapOutcome::NotStarted
                        },
                        rollback_error: None,
                    }),
                    unknown,
                );
            }
        };
        tx.set_drop_behavior(DropBehavior::Ignore);
        let captured = resources
            .capture_probes_in(&tx, previous, probes, identities)
            .map_err(|error| match error {
                super::CaptureError::Shape(reason) => {
                    BootstrapProblem::binding(BootstrapPhaseV1::LegacyHeader, reason)
                }
                error => BootstrapProblem::capture(
                    BootstrapPhaseV1::LegacyHeader,
                    FailureContext::Network,
                    error,
                ),
            });
        settle(db, captured, tx.rollback())
    })
}

/// Only settlement is shared. This does not execute an operation, start another
/// transaction, own a runner or turn a failed rollback into a durable result.
fn settle<T>(
    db: &Connection,
    captured: Result<T, BootstrapProblem>,
    rollback: rusqlite::Result<()>,
) -> (Result<T, BootstrapFailure>, bool) {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => {
            return (
                captured.map_err(|problem| BootstrapFailure {
                    problem,
                    // R0/R1 are read-only and the mutating T1 has not started.
                    outcome: BootstrapOutcome::NotStarted,
                    rollback_error: None,
                }),
                false,
            );
        }
        Err(error) => SafeDatabaseError::sql("rollback", &error),
        Ok(()) => SafeDatabaseError::invariant("nal_bootstrap_transaction_unsettled", "rollback"),
    };
    let failure = match captured {
        Ok(_) => BootstrapFailure {
            problem: BootstrapProblem {
                phase: BootstrapPhaseV1::LegacyHeader,
                context: FailureContext::Network,
                cause: BootstrapCause::Database(Box::new(rollback_error)),
            },
            outcome: BootstrapOutcome::Unknown {
                observed_revision: None,
            },
            rollback_error: None,
        },
        Err(problem) => BootstrapFailure {
            problem,
            outcome: BootstrapOutcome::Unknown {
                observed_revision: None,
            },
            rollback_error: Some(Box::new(rollback_error)),
        },
    };
    (Err(failure), true)
}

fn database_problem(phase: &'static str, error: &rusqlite::Error) -> BootstrapProblem {
    BootstrapProblem {
        phase: BootstrapPhaseV1::LegacyHeader,
        context: FailureContext::Network,
        cause: BootstrapCause::Database(Box::new(SafeDatabaseError::sql(phase, error))),
    }
}
