use super::*;
use research_agent_core::{
    context::checkpoint_store::CheckpointStoreError, model::ContextPersistenceDiagnostic,
};

#[test]
fn commit_failure_fences_owner_and_preserves_history_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let log = root.path().join("log");
    let (sql, batch) = setup_on(Connection::open(&path).unwrap());
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after insert on agent_context_fragments begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &log).unwrap();
    {
        let mut guard = db.lock().unwrap();
        assert!(
            matches!(managed::record(&mut guard, &batch), Err(CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "commit", safe_cause: "sql_commit_outcome_unknown", sqlite_extended_code: Some(787),
        })) if conversation_id == batch.conversation_id)
        );
    }
    assert!(db.lock().is_err());
    drop(db);
    let reopened =
        crate::data_interface::Database::open(Connection::open(&path).unwrap(), &log).unwrap();
    let guard = reopened.lock().unwrap();
    assert!(
        read(&guard, batch.conversation_id, batch.window_id, 0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        context_windows::read_projection(&guard, batch.conversation_id)
            .unwrap()
            .seq(),
        batch.boundary_seq
    );
}

#[test]
fn exact_replay_and_confirmed_rollback_leave_owner_available() {
    let root = tempfile::tempdir().unwrap();
    let (sql, batch) = setup_on(Connection::open(root.path().join("data.sqlite")).unwrap());
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    {
        let mut guard = db.lock().unwrap();
        let mut invalid = batch.clone();
        invalid.boundary_seq += 1;
        assert!(matches!(
            managed::record(&mut guard, &invalid),
            Err(CheckpointStoreError::Conflict)
        ));
    }
    {
        let mut guard = db.lock().unwrap();
        assert_eq!(managed::record(&mut guard, &batch), Ok(Recording::Recorded));
        assert_eq!(
            managed::record(&mut guard, &batch),
            Ok(Recording::AlreadyRecorded)
        );
    }
    assert!(db.lock().is_ok());
}
