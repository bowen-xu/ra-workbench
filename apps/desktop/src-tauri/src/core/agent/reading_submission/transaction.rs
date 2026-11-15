//! Guard the legacy paper preparation transaction until it gains an owned receipt.
#[cfg(test)]
use super::Error;
#[cfg(test)]
use crate::data_interface::DatabaseGuard;
#[cfg(test)]
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unknown {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub phase: &'static str,
    pub safe_cause: &'static str,
    pub sqlite_code: Option<String>,
    pub sqlite_extended_code: Option<i32>,
}

#[cfg(test)]
fn unknown(
    conversation: Uuid,
    request: Uuid,
    phase: &'static str,
    cause: &'static str,
    error: Option<&rusqlite::Error>,
) -> Error {
    let (sqlite_code, sqlite_extended_code) = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => {
            (Some(format!("{:?}", code.code)), Some(code.extended_code))
        }
        _ => (None, None),
    };
    Error::OutcomeUnknown(Unknown {
        conversation_id: conversation,
        request_id: request,
        phase,
        safe_cause: cause,
        sqlite_code,
        sqlite_extended_code,
    })
}

#[cfg(test)]
pub(super) fn prepare<T>(
    guard: &mut DatabaseGuard<'_>,
    conversation: Uuid,
    request: Uuid,
    body: impl FnOnce(&Transaction<'_>) -> Result<T, Error>,
) -> Result<T, Error> {
    if !guard.is_autocommit() {
        guard.require_recovery();
        return Err(unknown(
            conversation,
            request,
            "begin",
            "sql_transaction_already_open",
            None,
        ));
    }
    let (result, uncertain) = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(guard, conversation, request, body)
    })) {
        Ok(outcome) => outcome,
        Err(payload) => {
            guard.require_recovery();
            std::panic::resume_unwind(payload);
        }
    };
    if uncertain {
        guard.require_recovery();
    }
    result
}

#[cfg(test)]
fn run<T>(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    body: impl FnOnce(&Transaction<'_>) -> Result<T, Error>,
) -> (Result<T, Error>, bool) {
    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(error) if !db.is_autocommit() => {
            return (
                Err(unknown(
                    conversation,
                    request,
                    "begin",
                    "sql_begin_failed",
                    Some(&error),
                )),
                true,
            );
        }
        Err(_) => return (Err(Error::Unavailable), false),
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    match body(&tx) {
        Err(error) => {
            let rollback = tx.rollback();
            if rollback.is_err() || !db.is_autocommit() {
                return (
                    Err(unknown(
                        conversation,
                        request,
                        "rollback",
                        if rollback.is_err() {
                            "sql_rollback_failed"
                        } else {
                            "sql_transaction_remains_open"
                        },
                        rollback.as_ref().err(),
                    )),
                    true,
                );
            }
            (Err(error), false)
        }
        Ok(value) => {
            if let Err(error) = tx.commit() {
                let cleanup_failed = !db.is_autocommit()
                    && (db.execute_batch("rollback").is_err() || !db.is_autocommit());
                return (
                    Err(unknown(
                        conversation,
                        request,
                        "commit",
                        if cleanup_failed {
                            "sql_commit_unknown_cleanup_failed"
                        } else {
                            "sql_commit_outcome_unknown"
                        },
                        Some(&error),
                    )),
                    true,
                );
            }
            if !db.is_autocommit() {
                return (
                    Err(unknown(
                        conversation,
                        request,
                        "commit",
                        "sql_commit_left_transaction_open",
                        None,
                    )),
                    true,
                );
            }
            (Ok(value), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_interface::Database;

    #[test]
    fn rejected_preparation_rolls_back_without_blocking_owner() {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        sql.execute_batch("create table fixture_paper_value(value text)")
            .unwrap();
        let db = Database::open(sql, &root.path().join("log")).unwrap();
        let mut guard = db.lock().unwrap();
        let result: Result<(), Error> = prepare(&mut guard, Uuid::new_v4(), Uuid::new_v4(), |tx| {
            tx.execute("insert into fixture_paper_value values('temporary')", [])
                .unwrap();
            Err(Error::Conflict)
        });
        assert_eq!(result, Err(Error::Conflict));
        drop(guard);
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row("select count(*) from fixture_paper_value", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn panicking_worker_blocks_owner_before_guard_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let db = Database::open(sql, &root.path().join("log")).unwrap();
        let mut guard = db.lock().unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), Error> = prepare(&mut guard, Uuid::new_v4(), Uuid::new_v4(), |_| {
                panic!("synthetic worker panic")
            });
        }));
        assert!(panic.is_err());
        drop(guard);
        assert!(db.lock().is_err());
    }

    #[test]
    fn uncertain_paper_commit_blocks_owner_and_preserves_identity_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("data.sqlite");
        let log = root.path().join("log");
        let mut sql = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        sql.execute_batch("create table fixture_paper_parent(id integer primary key); create table fixture_paper_value(id integer references fixture_paper_parent(id) deferrable initially deferred);").unwrap();
        let db = Database::open(sql, &log).unwrap();
        let conversation = Uuid::new_v4();
        let request = Uuid::new_v4();
        let mut guard = db.lock().unwrap();
        let error = prepare(&mut guard, conversation, request, |tx| {
            tx.execute("insert into fixture_paper_value values(1)", [])
                .unwrap();
            Ok(())
        })
        .unwrap_err();
        let Error::OutcomeUnknown(detail) = error else {
            panic!("expected unknown commit")
        };
        assert_eq!(
            (
                detail.conversation_id,
                detail.request_id,
                detail.phase,
                detail.safe_cause
            ),
            (
                conversation,
                request,
                "commit",
                "sql_commit_outcome_unknown"
            )
        );
        assert_eq!(detail.sqlite_extended_code, Some(787));
        drop(guard);
        assert!(db.lock().is_err());
        drop(db);
        let reopened = Database::open(Connection::open(&path).unwrap(), &log).unwrap();
        assert_eq!(
            reopened
                .lock()
                .unwrap()
                .query_row("select count(*) from fixture_paper_value", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}
