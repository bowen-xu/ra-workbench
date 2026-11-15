use super::{
    CommittedCatalogRows, SafeDatabaseError, SaveCause, SaveFailure, SaveOutcome, SaveResources,
};
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction},
    runtime_plugins::literature_catalog::data::{CatalogResources, capture_items},
};
use research_workbench_native_sdk::catalog_save::CatalogSavePlan;
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

/// Preserve BEGIN failure precedence before releasing the guard for preparation.
pub(crate) fn probe_begin(db: &mut DatabaseGuard<'_>) -> Result<(), SaveFailure> {
    guarded_transaction::run(db, |connection| {
        let was_autocommit = connection.is_autocommit();
        if let Err(cause) = connection.execute_batch("BEGIN DEFERRED") {
            let unknown = was_autocommit && !connection.is_autocommit();
            return (
                Err(SaveFailure::database(
                    SafeDatabaseError::sql("begin", &cause),
                    if unknown {
                        SaveOutcome::Unknown
                    } else {
                        SaveOutcome::NotStarted
                    },
                )),
                unknown,
            );
        }
        match connection.execute_batch("ROLLBACK") {
            Ok(()) if connection.is_autocommit() => (Ok(()), false),
            result => {
                let cause = match result {
                    Err(cause) => SafeDatabaseError::sql("rollback", &cause),
                    Ok(()) => SafeDatabaseError::invariant(
                        "catalog_save_transaction_unsettled",
                        "rollback",
                    ),
                };
                (
                    Err(SaveFailure::database(cause, SaveOutcome::Unknown)),
                    true,
                )
            }
        }
    })
}

pub(crate) fn persist(
    db: &mut DatabaseGuard<'_>,
    resources: &SaveResources,
    catalog: &CatalogResources,
    plan: &CatalogSavePlan,
) -> Result<CommittedCatalogRows, SaveFailure> {
    if resources.package_key() != catalog.package_key() {
        return Err(SaveFailure::database(
            SafeDatabaseError::invariant("plugin_data_resource_mismatch", "scope"),
            SaveOutcome::NotStarted,
        ));
    }
    guarded_transaction::run(db, |connection| run(connection, resources, catalog, plan))
}

fn run(
    connection: &Connection,
    resources: &SaveResources,
    catalog: &CatalogResources,
    plan: &CatalogSavePlan,
) -> (Result<CommittedCatalogRows, SaveFailure>, bool) {
    let was_autocommit = connection.is_autocommit();
    let mut tx = match Transaction::new_unchecked(connection, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(cause) => {
            let unknown = was_autocommit && !connection.is_autocommit();
            return (
                Err(SaveFailure::database(
                    SafeDatabaseError::sql("begin", &cause),
                    if unknown {
                        SaveOutcome::Unknown
                    } else {
                        SaveOutcome::NotStarted
                    },
                )),
                unknown,
            );
        }
    };
    // Explicitly observe rollback/commit; panic is fenced by guarded_transaction.
    tx.set_drop_behavior(DropBehavior::Ignore);
    let item_id = plan
        .item
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Err(cause) = super::steps::save(&tx, resources, plan, &item_id) {
        let rollback = tx.rollback();
        return failed(connection, SaveCause::Data(cause), rollback);
    }
    if let Err(cause) = tx.commit() {
        let cause = SaveCause::Database(Box::new(SafeDatabaseError::sql("commit", &cause)));
        // A failed COMMIT with no remaining transaction is not a rollback receipt.
        if connection.is_autocommit() {
            return (
                Err(SaveFailure {
                    cause,
                    outcome: SaveOutcome::Unknown,
                    rollback_error: None,
                }),
                true,
            );
        }
        return failed(connection, cause, connection.execute_batch("ROLLBACK"));
    }
    if !connection.is_autocommit() {
        return (
            Err(SaveFailure::database(
                SafeDatabaseError::invariant("catalog_save_transaction_unsettled", "commit"),
                SaveOutcome::Unknown,
            )),
            true,
        );
    }
    // Original item_from ran after commit under this same guard. Missing visible
    // main rows remain owned raw rows for the domain's original not_found outcome.
    match capture_items(connection, catalog, std::slice::from_ref(&item_id)) {
        Ok(items) => (Ok(CommittedCatalogRows { item_id, items }), false),
        Err(cause) => (
            Err(SaveFailure {
                cause: SaveCause::Data(cause),
                outcome: SaveOutcome::Committed { item_id },
                rollback_error: None,
            }),
            false,
        ),
    }
}

fn failed<T>(
    connection: &Connection,
    cause: SaveCause,
    rollback: rusqlite::Result<()>,
) -> (Result<T, SaveFailure>, bool) {
    let (outcome, rollback_error) = match rollback {
        Ok(()) if connection.is_autocommit() => (SaveOutcome::RolledBack, None),
        Err(error) => (
            SaveOutcome::Unknown,
            Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
        ),
        Ok(()) => (
            SaveOutcome::Unknown,
            Some(Box::new(SafeDatabaseError::invariant(
                "catalog_save_transaction_unsettled",
                "rollback",
            ))),
        ),
    };
    let unknown = outcome == SaveOutcome::Unknown;
    (
        Err(SaveFailure {
            cause,
            outcome,
            rollback_error,
        }),
        unknown,
    )
}
