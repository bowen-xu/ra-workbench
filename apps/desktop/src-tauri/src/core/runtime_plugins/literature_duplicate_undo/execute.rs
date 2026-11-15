use super::{
    CapturedUndo, UndoCause, UndoFailure, UndoOutcome, UndoPhase as Phase, UndoProblem,
    UndoResources, load,
};
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction, plugin_resources::write_in},
    runtime_plugins::{data::AcceptedDataResource, native::SafeDatabaseError},
};
use research_workbench_native_sdk::{data::DataValue, duplicate_undo::PreparedDuplicateUndoV1};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

mod inner;
mod restore;

type Result<T> = std::result::Result<T, UndoProblem>;

struct Execution<'a, 'connection> {
    tx: &'a Transaction<'connection>,
    resources: &'a UndoResources,
    captured: &'a CapturedUndo,
    plan: &'a PreparedDuplicateUndoV1,
}

impl Execution<'_, '_> {
    fn write(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
    ) -> Result<()> {
        write_in(self.tx, resource, &load::call(resource, arguments))
            .map(|_| ())
            .map_err(|error| UndoProblem::data(phase, error))
    }
    fn run(&self) -> Result<()> {
        self.inner()?;
        self.restore()
    }
}

/// SDK encodings and the immutable snapshot identity bind Native's reply.
/// No current database state is predicted, rechecked against T0, or sent here.
pub(crate) fn check_plan(captured: &CapturedUndo, plan: &PreparedDuplicateUndoV1) -> Result<()> {
    let hash = format!("{:x}", Sha256::digest(captured.snapshot_json.as_bytes()));
    if hash != plan.snapshot_sha256 {
        return Err(UndoProblem::binding(
            Phase::Preparation,
            "snapshot-identity",
        ));
    }
    let mismatch = || UndoProblem::binding(Phase::Preparation, "snapshot-layout");
    match (&captured.snapshot.classification, &plan.classification) {
        (None, None) => Ok(()),
        (Some(snapshot), Some(plan)) => {
            if load::encode(&snapshot.after_memberships, Phase::Preparation)?
                != plan.expected_memberships_json
                || load::encode(&snapshot.after_assignments, Phase::Preparation)?
                    != plan.expected_assignments_json
            {
                return Err(mismatch());
            }
            match (&snapshot.nal, &plan.nal) {
                (None, None) => Ok(()),
                (Some(snapshot), Some(plan))
                    if load::encode(&snapshot.after, Phase::Preparation)?
                        == plan.expected_statements_json
                        && load::encode(&snapshot.after_annotations, Phase::Preparation)?
                            == plan.expected_annotations_json =>
                {
                    Ok(())
                }
                _ => Err(mismatch()),
            }
        }
        _ => Err(mismatch()),
    }
}

pub(crate) fn apply(
    guard: &mut DatabaseGuard<'_>,
    resources: &UndoResources,
    captured: &CapturedUndo,
    plan: &PreparedDuplicateUndoV1,
) -> std::result::Result<(), UndoFailure> {
    if let Err(problem) = check_plan(captured, plan) {
        return Err(UndoFailure {
            problem,
            outcome: UndoOutcome::NotStarted,
            rollback_error: None,
        });
    }
    guarded_transaction::run(guard, |db| run(db, resources, captured, plan))
}

fn run(
    db: &Connection,
    resources: &UndoResources,
    captured: &CapturedUndo,
    plan: &PreparedDuplicateUndoV1,
) -> (std::result::Result<(), UndoFailure>, bool) {
    let was_autocommit = db.is_autocommit();
    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(error) => {
            let unknown = was_autocommit && !db.is_autocommit();
            return (
                Err(UndoFailure {
                    problem: UndoProblem::sql(Phase::Begin, "begin", &error),
                    outcome: if unknown {
                        UndoOutcome::Unknown
                    } else {
                        UndoOutcome::NotStarted
                    },
                    rollback_error: None,
                }),
                unknown,
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let result = Execution {
        tx: &tx,
        resources,
        captured,
        plan,
    }
    .run();
    if let Err(problem) = result {
        return failed(db, problem, tx.rollback());
    }
    if let Err(error) = tx.commit() {
        let problem = UndoProblem::sql(Phase::Commit, "commit", &error);
        if db.is_autocommit() {
            return (
                Err(UndoFailure {
                    problem,
                    outcome: UndoOutcome::Unknown,
                    rollback_error: None,
                }),
                true,
            );
        }
        return failed(db, problem, db.execute_batch("ROLLBACK"));
    }
    if !db.is_autocommit() {
        return (
            Err(UndoFailure {
                problem: UndoProblem {
                    phase: Phase::Commit,
                    cause: UndoCause::Database(Box::new(SafeDatabaseError::invariant(
                        "duplicate_undo_transaction_unsettled",
                        "commit",
                    ))),
                },
                outcome: UndoOutcome::Unknown,
                rollback_error: None,
            }),
            true,
        );
    }
    (Ok(()), false)
}

fn failed(
    db: &Connection,
    problem: UndoProblem,
    rollback: rusqlite::Result<()>,
) -> (std::result::Result<(), UndoFailure>, bool) {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Err(error) => Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "duplicate_undo_transaction_unsettled",
            "rollback",
        ))),
    };
    let unknown = rollback_error.is_some();
    (
        Err(UndoFailure {
            problem,
            outcome: if unknown {
                UndoOutcome::Unknown
            } else {
                UndoOutcome::RolledBack
            },
            rollback_error,
        }),
        unknown,
    )
}
