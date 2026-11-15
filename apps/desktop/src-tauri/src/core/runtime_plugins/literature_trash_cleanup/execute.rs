use super::{
    CheckedCleanupPlan, TrashCleanupFailure as Failure, TrashCleanupOutcome as Outcome,
    TrashCleanupProblem as Problem, TrashCleanupResources as Resources,
};
use crate::{
    data_interface::{
        DatabaseGuard, guarded_transaction,
        plugin_resources::{ColumnCodec, RowPolicy},
    },
    runtime_plugins::native::SafeDatabaseError,
};
use research_workbench_native_sdk::data::{DataRows, DataValue};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

mod cleanup;

type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug, PartialEq, Eq)]
struct DeletionStats {
    scheduled_content_count: usize,
    estimated_reclaimable_bytes: i64,
    retained_shared_content_count: usize,
    cleanup_warnings: Vec<String>,
}

pub(crate) fn purge(
    guard: &mut DatabaseGuard<'_>,
    resources: &Resources,
    plan: &CheckedCleanupPlan,
) -> Result<Vec<String>> {
    guarded_transaction::run(guard, |db| {
        let result = purge_in(db, resources, plan);
        let unknown = result
            .as_ref()
            .is_err_and(|error| error.outcome == Outcome::Unknown);
        (result, unknown)
    })
}

fn purge_in(
    db: &Connection,
    resources: &Resources,
    plan: &CheckedCleanupPlan,
) -> Result<Vec<String>> {
    let ids = resources
        .read(
            db,
            "expired-ids",
            vec![],
            RowPolicy::All,
            &[ColumnCodec::String],
        )
        .map_err(|problem| Failure::new("expired-ids", problem, Outcome::NotStarted))?;
    // Capture the full vector before the first transaction, including original row conversions.
    let ids: Vec<String> = ids
        .rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [DataValue::Text(id)] => Ok(id.clone()),
            _ => Err(Failure::new(
                "expired-ids",
                Problem::Binding,
                Outcome::NotStarted,
            )),
        })
        .collect::<Result<_>>()?;
    let mut committed = Vec::new();
    let mut warnings = Vec::new();
    for item in ids {
        match delete_one(db, resources, plan, &item) {
            Ok(stats) => {
                committed.push(item);
                warnings.extend(stats.cleanup_warnings);
            }
            Err(mut failure) => {
                failure.item_id = Some(item);
                failure.committed_item_ids = committed;
                return Err(failure);
            }
        }
    }
    Ok(warnings)
}

fn delete_one(
    db: &Connection,
    resources: &Resources,
    plan: &CheckedCleanupPlan,
    item: &str,
) -> Result<DeletionStats> {
    let shared_count = impact(db, resources, item)
        .map_err(|problem| Failure::new("impact", problem, Outcome::NotStarted))?;
    let was_autocommit = db.is_autocommit();
    let mut tx =
        Transaction::new_unchecked(db, TransactionBehavior::Deferred).map_err(|error| {
            Failure::new(
                "begin",
                Problem::Database(Box::new(SafeDatabaseError::sql("begin", &error))),
                if was_autocommit && !db.is_autocommit() {
                    Outcome::Unknown
                } else {
                    Outcome::NotStarted
                },
            )
        })?;
    tx.set_drop_behavior(DropBehavior::Ignore);
    let stats = match steps(&tx, resources, plan, item, shared_count) {
        Ok(stats) => stats,
        Err((phase, problem)) => return Err(failed(db, phase, problem, tx.rollback())),
    };
    if let Err(error) = tx.commit() {
        let problem = Problem::Database(Box::new(SafeDatabaseError::sql("commit", &error)));
        if db.is_autocommit() {
            return Err(Failure::new("commit", problem, Outcome::Unknown));
        }
        return Err(failed(db, "commit", problem, db.execute_batch("ROLLBACK")));
    }
    if !db.is_autocommit() {
        return Err(Failure::new(
            "commit",
            Problem::Owner("connection_unsettled"),
            Outcome::Unknown,
        ));
    }
    Ok(stats)
}

fn impact(
    db: &Connection,
    resources: &Resources,
    item: &str,
) -> std::result::Result<usize, Problem> {
    let exists = resources.read(
        db,
        "item-exists",
        text(item),
        RowPolicy::FirstRequired,
        &[ColumnCodec::Bool],
    )?;
    if integer(&exists, 0, 0)? == 0 {
        return Err(Problem::NotFound);
    }
    let rows = resources.read(
        db,
        "deletion-impact",
        text(item),
        RowPolicy::FirstRequired,
        &[ColumnCodec::U64; 6],
    )?;
    let mut values = [0usize; 6];
    for (index, value) in values.iter_mut().enumerate() {
        *value = usize::try_from(integer(&rows, 0, index)?).map_err(|_| Problem::Binding)?;
    }
    Ok(values[5])
}

fn steps(
    tx: &Transaction<'_>,
    resources: &Resources,
    plan: &CheckedCleanupPlan,
    item: &str,
    retained_shared_content_count: usize,
) -> std::result::Result<DeletionStats, (&'static str, Problem)> {
    let contents = resources
        .read(
            tx,
            "content-objects",
            text(item),
            RowPolicy::All,
            &[ColumnCodec::String, ColumnCodec::I64],
        )
        .map_err(|p| ("content-objects", p))?;
    let contents: Vec<(String, i64)> = contents
        .rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [DataValue::Text(hash), DataValue::Integer(size)] => Ok((hash.clone(), *size)),
            _ => Err(("content-objects", Problem::Binding)),
        })
        .collect::<std::result::Result<_, _>>()?;
    cleanup::remove(tx, resources, plan, item)?;
    resources
        .write(tx, "fts-delete", text(item))
        .map_err(|p| ("fts-delete", p))?;
    let affected = resources
        .write(tx, "item-delete", text(item))
        .map_err(|p| ("item-delete", p))?;
    if affected == 0 {
        return Err(("item-delete", Problem::NotFound));
    }
    let mut stats = DeletionStats {
        scheduled_content_count: 0,
        estimated_reclaimable_bytes: 0,
        retained_shared_content_count,
        cleanup_warnings: Vec::new(),
    };
    for (hash, size) in contents {
        let rows = resources
            .read(
                tx,
                "gc-scheduled",
                text(&hash),
                RowPolicy::FirstRequired,
                &[ColumnCodec::Bool],
            )
            .map_err(|p| ("gc-scheduled", p))?;
        if integer(&rows, 0, 0).map_err(|p| ("gc-scheduled", p))? != 0 {
            stats.scheduled_content_count += 1;
            stats.estimated_reclaimable_bytes =
                stats.estimated_reclaimable_bytes.saturating_add(size);
        }
    }
    resources
        .write(tx, "orphan-creators-delete", vec![])
        .map_err(|p| ("orphan-creators", p))?;
    resources
        .write(tx, "orphan-tags-delete", vec![])
        .map_err(|p| ("orphan-tags", p))?;
    resources
        .write(tx, "finish", text(item))
        .map_err(|p| ("finish", p))?;
    Ok(stats)
}

fn failed(
    db: &Connection,
    phase: &'static str,
    problem: Problem,
    rollback: rusqlite::Result<()>,
) -> Failure {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "trash_cleanup_transaction_unsettled",
            "rollback",
        ))),
        Err(error) => Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
    };
    let mut failure = Failure::new(
        phase,
        problem,
        if rollback_error.is_some() {
            Outcome::Unknown
        } else {
            Outcome::RolledBack
        },
    );
    failure.rollback_error = rollback_error;
    failure
}

fn text(value: &str) -> Vec<DataValue> {
    vec![DataValue::Text(value.into())]
}
fn integer(rows: &DataRows, row: usize, column: usize) -> std::result::Result<i64, Problem> {
    match rows.rows.get(row).and_then(|row| row.get(column)) {
        Some(DataValue::Integer(value)) => Ok(*value),
        _ => Err(Problem::Binding),
    }
}
