//! Read-prefix pauses leave no SQL transaction or unconfirmed owner behind.
use super::*;
use crate::agent::writing_context::{ContextRead, WritingContextRequest};
use research_agent_core::submission::ContextText;
use research_workbench_native_sdk::writing_project::Input;
use std::path::Path;

struct UnusedReader;
impl WritingContextReader for UnusedReader {
    fn selected(
        &mut self,
        _: &Connection,
        _: &Path,
        _: WritingContextRequest,
    ) -> ContextRead<Vec<ContextText>> {
        panic!("callback owns the test read")
    }
    fn current_file(
        &mut self,
        _: &Connection,
        _: &Path,
        _: Uuid,
        _: &str,
    ) -> ContextRead<Vec<ContextText>> {
        panic!("callback owns the test read")
    }
}
fn open(root: &Path) -> crate::data_interface::SharedDatabase {
    let mut sql = Connection::open(root.join("read.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("log")).unwrap()
}
fn pause() -> Result<(), Flow<WritingContextError, ContextNeed>> {
    Err(Flow::Need(Box::new(Input::PrepareStatus {
        manuscript_id: Uuid::new_v4().to_string(),
    })))
}

#[test]
fn complete_failure_and_need_each_release_the_original_transaction() {
    let root = tempfile::tempdir().unwrap();
    let database = open(root.path());
    let operation = Uuid::new_v4();
    let complete = database
        .read_writing_context(
            operation,
            ContextReadMode::Snapshot,
            &mut UnusedReader,
            |sql, _| {
                assert!(!sql.is_autocommit());
                Ok::<_, Flow<WritingContextError, ContextNeed>>(17)
            },
        )
        .unwrap();
    assert_eq!(complete, 17);
    assert!(database.lock().unwrap().is_autocommit());
    let need = database.read_writing_context(
        operation,
        ContextReadMode::Snapshot,
        &mut UnusedReader,
        |sql, _| {
            assert!(!sql.is_autocommit());
            pause()
        },
    );
    assert!(matches!(need, Err(Flow::Need(_))));
    assert!(database.lock().unwrap().is_autocommit());
    let failed: Result<(), _> = database.read_writing_context(
        operation,
        ContextReadMode::Snapshot,
        &mut UnusedReader,
        |_, _| Err(Flow::original(WritingContextError::Conflict)),
    );
    assert!(matches!(
        failed,
        Err(Flow::Fail(Failure {
            error: SqlImmediateFailure::Unavailable(_),
            ..
        }))
    ));
    assert!(database.lock().unwrap().is_autocommit());
}

#[test]
fn unconfirmed_rollback_blocks_need_and_fences_the_owner_until_reopen() {
    let root = tempfile::tempdir().unwrap();
    let database = open(root.path());
    let operation = Uuid::new_v4();
    let result = database.read_writing_context(
        operation,
        ContextReadMode::Snapshot,
        &mut UnusedReader,
        |sql, _| {
            // Inject an independently ended transaction, forcing explicit settlement to fail.
            sql.execute_batch("rollback").unwrap();
            pause()
        },
    );
    match result {
        Err(Flow::Fail(Failure {
            error: SqlImmediateFailure::Unknown(detail),
            ..
        })) => {
            assert_eq!(detail.operation_id, operation);
            assert!(detail.sqlite_extended_code.is_some());
        }
        _ => panic!("an unconfirmed rollback must override Need"),
    }
    assert!(database.lock().is_err());
    drop(database);
    assert!(open(root.path()).lock().unwrap().is_autocommit());
}

#[test]
fn mixed_core_read_proofs_remain_committed_before_need_and_original_failure() {
    let root = tempfile::tempdir().unwrap();
    let database = open(root.path());
    database
        .lock()
        .unwrap()
        .execute_batch("create table fixture_core_read_proofs(id integer primary key)")
        .unwrap();
    let operation = Uuid::new_v4();
    let need = database.read_writing_context(
        operation,
        ContextReadMode::CoreProofs,
        &mut UnusedReader,
        |sql, _| {
            assert!(sql.is_autocommit());
            sql.execute("insert into fixture_core_read_proofs values(1)", [])
                .unwrap();
            pause()
        },
    );
    assert!(matches!(need, Err(Flow::Need(_))));
    let count: u64 = database
        .lock()
        .unwrap()
        .query_row("select count(*) from fixture_core_read_proofs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    let failed: Result<(), _> = database.read_writing_context(
        operation,
        ContextReadMode::CoreProofs,
        &mut UnusedReader,
        |sql, _| {
            assert!(sql.is_autocommit());
            sql.execute("insert into fixture_core_read_proofs values(2)", [])
                .unwrap();
            Err(Flow::original(WritingContextError::Conflict))
        },
    );
    assert!(matches!(failed, Err(Flow::Fail(_))));
    let sql = database.lock().unwrap();
    assert!(sql.is_autocommit());
    let count: u64 = sql
        .query_row("select count(*) from fixture_core_read_proofs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn mixed_unsettled_core_read_owner_blocks_native_need_and_requires_reopen() {
    let root = tempfile::tempdir().unwrap();
    let database = open(root.path());
    let result = database.read_writing_context(
        Uuid::new_v4(),
        ContextReadMode::CoreProofs,
        &mut UnusedReader,
        |sql, _| {
            sql.execute_batch("begin immediate").unwrap();
            pause()
        },
    );
    assert!(matches!(
        result,
        Err(Flow::Fail(Failure {
            error: SqlImmediateFailure::Unknown(_),
            ..
        }))
    ));
    assert!(database.lock().is_err());
    drop(database);
    let reopened = open(root.path());
    assert!(reopened.lock().unwrap().is_autocommit());
}
