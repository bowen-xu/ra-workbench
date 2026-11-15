use super::{
    LifecycleFailure, LifecycleOutcome as Outcome, LifecycleProblem as Problem, LifecycleResources,
};
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction},
    runtime_plugins::native::SafeDatabaseError,
};
use research_workbench_native_sdk::{
    data::DataValue, literature_lifecycle::LifecycleRequestV1 as Request,
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, LifecycleFailure>;

/// Verify the complete echo and that the batch is exactly an ordered unique
/// representation of the original IDs. Native, rather than Core, prepares it.
pub(crate) fn check_preparation(
    request: &Request,
    prepared: &Request,
    ids: &[String],
) -> Result<()> {
    let matches = request == prepared
        && match request {
            Request::TrashMany { item_ids } => {
                let expected: BTreeSet<_> = item_ids.iter().collect();
                !ids.is_empty() && ids.len() == expected.len() && ids.iter().eq(expected)
            }
            _ => ids.is_empty(),
        };
    if matches {
        Ok(())
    } else {
        Err(LifecycleFailure::new(
            "preparation",
            Problem::Binding,
            Outcome::NotDispatched,
        ))
    }
}

pub(crate) fn apply(
    guard: &mut DatabaseGuard<'_>,
    resources: &LifecycleResources,
    request: &Request,
    ids: &[String],
) -> Result<()> {
    check_preparation(request, request, ids)?;
    guarded_transaction::run(guard, |db| match request {
        Request::TrashMany { .. } | Request::SetProjects { .. } => {
            transaction(db, resources, request, ids)
        }
        _ => single(db, resources, request),
    })
}

fn single(
    db: &Connection,
    resources: &LifecycleResources,
    request: &Request,
) -> (Result<()>, bool) {
    if !db.is_autocommit() {
        return (
            Err(LifecycleFailure::new(
                "admission",
                Problem::Owner("connection_unsettled"),
                Outcome::Unknown,
            )),
            true,
        );
    }
    let (resource, id) = match request {
        Request::Archive { item_id } => (&resources.archive, item_id),
        Request::Restore { item_id } => (&resources.restore, item_id),
        Request::Trash { item_id } => (&resources.trash, item_id),
        Request::RestoreTrash { item_id } => (&resources.restore_trash, item_id),
        _ => {
            return (
                Err(LifecycleFailure::new(
                    "dispatch",
                    Problem::Binding,
                    Outcome::NotDispatched,
                )),
                false,
            );
        }
    };
    let written = resources.write(db, resource, &[DataValue::Text(id.clone())]);
    let settled = db.is_autocommit();
    let result = match written {
        Err(error) => {
            let state = if !settled {
                Outcome::Unknown
            } else if matches!(error.phase, "validate" | "prepare" | "parameters") {
                Outcome::NotDispatched
            } else {
                Outcome::StatementFailed
            };
            Err(LifecycleFailure::new(
                "statement",
                Problem::Data(error),
                state,
            ))
        }
        Ok(_) if !settled => Err(LifecycleFailure::new(
            "statement",
            Problem::Owner("connection_unsettled"),
            Outcome::Unknown,
        )),
        Ok(affected_rows) => checked_effect(db, resources, affected_rows).map_err(|problem| {
            LifecycleFailure::new(
                "statement-result",
                problem,
                Outcome::StatementReturned { affected_rows },
            )
        }),
    };
    // Preserve original autocommit FAIL effects. No added rollback or replay.
    (result, !settled)
}

fn checked_effect(
    db: &Connection,
    resources: &LifecycleResources,
    affected: u64,
) -> std::result::Result<(), Problem> {
    match resources.missing(db, affected).map_err(Problem::Data)? {
        None => Ok(()),
        Some(code) if code == "not_found" => Err(Problem::Missing),
        Some(_) => Err(Problem::Binding),
    }
}

fn transaction(
    db: &Connection,
    resources: &LifecycleResources,
    request: &Request,
    ids: &[String],
) -> (Result<()>, bool) {
    let was_autocommit = db.is_autocommit();
    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(error) => {
            let unknown = was_autocommit && !db.is_autocommit();
            return (
                Err(LifecycleFailure::new(
                    "begin",
                    Problem::Database(Box::new(SafeDatabaseError::sql("begin", &error))),
                    if unknown {
                        Outcome::Unknown
                    } else {
                        Outcome::NotDispatched
                    },
                )),
                unknown,
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let result = transaction_steps(&tx, resources, request, ids);
    if let Err(problem) = result {
        return failed(db, "transaction", problem, tx.rollback());
    }
    if let Err(error) = tx.commit() {
        let problem = Problem::Database(Box::new(SafeDatabaseError::sql("commit", &error)));
        if db.is_autocommit() {
            return (
                Err(LifecycleFailure::new("commit", problem, Outcome::Unknown)),
                true,
            );
        }
        return failed(db, "commit", problem, db.execute_batch("ROLLBACK"));
    }
    if !db.is_autocommit() {
        return (
            Err(LifecycleFailure::new(
                "commit",
                Problem::Owner("connection_unsettled"),
                Outcome::Unknown,
            )),
            true,
        );
    }
    (Ok(()), false)
}

fn transaction_steps(
    db: &Connection,
    resources: &LifecycleResources,
    request: &Request,
    ids: &[String],
) -> std::result::Result<(), Problem> {
    match request {
        Request::TrashMany { .. } => {
            for id in ids {
                let affected = resources
                    .write(db, &resources.trash, &[DataValue::Text(id.clone())])
                    .map_err(Problem::Data)?;
                checked_effect(db, resources, affected)?;
            }
        }
        Request::SetProjects {
            item_id,
            project_ids,
        } => {
            resources
                .write(
                    db,
                    &resources.project_delete,
                    &[DataValue::Text(item_id.clone())],
                )
                .map_err(Problem::Data)?;
            for project_id in project_ids {
                resources
                    .write(
                        db,
                        &resources.project_insert,
                        &[
                            DataValue::Text(project_id.clone()),
                            DataValue::Text(item_id.clone()),
                        ],
                    )
                    .map_err(Problem::Data)?;
            }
        }
        _ => return Err(Problem::Binding),
    }
    Ok(())
}

fn failed(
    db: &Connection,
    phase: &'static str,
    problem: Problem,
    rollback: rusqlite::Result<()>,
) -> (Result<()>, bool) {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "literature_lifecycle_unsettled",
            "rollback",
        ))),
        Err(error) => Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
    };
    let unknown = rollback_error.is_some();
    (
        Err(LifecycleFailure {
            phase,
            problem,
            outcome: if unknown {
                Outcome::Unknown
            } else {
                Outcome::RolledBack
            },
            rollback_error,
        }),
        unknown,
    )
}
