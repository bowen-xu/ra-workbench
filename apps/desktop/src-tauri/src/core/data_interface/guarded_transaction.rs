//! Fence compatibility transactions even when their body unwinds.
use crate::data_interface::DatabaseGuard;
use rusqlite::Connection;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

pub(crate) fn run<T, E>(
    guard: &mut DatabaseGuard<'_>,
    operation: impl FnOnce(&Connection) -> (Result<T, E>, bool),
) -> Result<T, E> {
    run_mut(guard, |db| operation(db))
}

/// Preserve the original mutable Connection API of compatibility migrations.
pub(crate) fn run_mut<T, E>(
    guard: &mut DatabaseGuard<'_>,
    operation: impl FnOnce(&mut Connection) -> (Result<T, E>, bool),
) -> Result<T, E> {
    match catch_unwind(AssertUnwindSafe(|| operation(guard))) {
        Ok((result, unknown)) => {
            if unknown {
                guard.require_recovery();
            }
            result
        }
        Err(payload) => {
            guard.require_recovery();
            resume_unwind(payload)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_interface::Database;
    use rusqlite::{DropBehavior, Transaction, TransactionBehavior};

    #[test]
    fn panic_with_an_open_transaction_blocks_owner_and_reopen_rolls_back() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("data.sqlite");
        let log = root.path().join("log");
        let mut sql = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        sql.execute_batch("create table fixture_panic(id integer primary key, value text not null); insert into fixture_panic values(1,'before');").unwrap();
        let db = Database::open(sql, &log).unwrap();
        let mut guard = db.lock().unwrap();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _: Result<(), ()> = run(&mut guard, |connection| {
                let mut transaction =
                    Transaction::new_unchecked(connection, TransactionBehavior::Deferred).unwrap();
                transaction.set_drop_behavior(DropBehavior::Ignore);
                transaction
                    .execute("update fixture_panic set value='after' where id=1", [])
                    .unwrap();
                panic!("synthetic transaction panic");
            });
        }));
        assert!(outcome.is_err());
        assert!(!guard.is_autocommit());
        drop(guard);
        assert!(
            db.lock().is_err(),
            "panic must retain the original owner barrier"
        );
        drop(db);
        let sql = Connection::open(&path).unwrap();
        let reopened = Database::open(sql, &log).unwrap();
        let guard = reopened.lock().unwrap();
        let value: String = guard
            .query_row("select value from fixture_panic where id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, "before");
    }
}
