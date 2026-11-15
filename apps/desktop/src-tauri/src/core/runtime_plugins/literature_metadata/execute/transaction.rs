use super::*;
use rusqlite::{DropBehavior, TransactionBehavior};

pub(super) fn run(
    db: &Connection,
    steps: impl FnOnce(&Transaction<'_>) -> StepResult<()>,
) -> Result<()> {
    let was_autocommit = db.is_autocommit();
    let mut tx =
        Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|error| {
            MetadataFailure::new(
                "begin",
                MetadataProblem::Database(Box::new(SafeDatabaseError::sql("begin", &error))),
                if was_autocommit && !db.is_autocommit() {
                    MetadataPhaseOutcome::Unknown
                } else {
                    MetadataPhaseOutcome::NotStarted
                },
            )
        })?;
    tx.set_drop_behavior(DropBehavior::Ignore);
    if let Err((phase, problem)) = steps(&tx) {
        return Err(failed(db, phase, problem, tx.rollback()));
    }
    if let Err(error) = tx.commit() {
        let problem = MetadataProblem::Database(Box::new(SafeDatabaseError::sql("commit", &error)));
        if db.is_autocommit() {
            return Err(MetadataFailure::new(
                "commit",
                problem,
                MetadataPhaseOutcome::Unknown,
            ));
        }
        return Err(failed(db, "commit", problem, db.execute_batch("ROLLBACK")));
    }
    if !db.is_autocommit() {
        return Err(MetadataFailure::new(
            "commit",
            MetadataProblem::Owner("connection_unsettled"),
            MetadataPhaseOutcome::Unknown,
        ));
    }
    Ok(())
}

fn failed(
    db: &Connection,
    phase: &'static str,
    problem: MetadataProblem,
    rollback: rusqlite::Result<()>,
) -> MetadataFailure {
    let (outcome, rollback_error) = match rollback {
        Ok(()) if db.is_autocommit() => (MetadataPhaseOutcome::RolledBack, None),
        Err(error) => (
            MetadataPhaseOutcome::Unknown,
            Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
        ),
        Ok(()) => (
            MetadataPhaseOutcome::Unknown,
            Some(Box::new(SafeDatabaseError::invariant(
                "metadata_transaction_unsettled",
                "rollback",
            ))),
        ),
    };
    let mut failure = MetadataFailure::new(phase, problem, outcome);
    failure.rollback_error = rollback_error;
    failure
}
