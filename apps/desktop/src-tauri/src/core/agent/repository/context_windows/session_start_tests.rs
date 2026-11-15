use super::*;

#[test]
fn managed_consume_commit_failure_blocks_owner_and_reopen_keeps_pending() {
    use research_agent_core::{
        context::checkpoint_store::CheckpointStoreError, model::ContextPersistenceDiagnostic,
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let log = root.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    let saved = super::super::tests::fixture(&mut sql);
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on").unwrap();
    install(&mut sql, &saved).unwrap();
    sql.execute_batch("create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after delete on agent_compact_session_start begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &log).unwrap();
    {
        let mut guard = db.lock().unwrap();
        assert!(
            matches!(take_managed(&mut guard, saved.conversation_id), Err(CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "commit", safe_cause: "sql_commit_outcome_unknown", sqlite_extended_code: Some(787),
        })) if conversation_id == saved.conversation_id)
        );
    }
    assert!(db.lock().is_err());
    drop(db);
    let sql = Connection::open(&path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log).unwrap();
    let guard = reopened.lock().unwrap();
    let pending: String = guard
        .query_row(
            "select window_id from agent_compact_session_start where conversation_id=?1",
            [saved.conversation_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, saved.window_id.to_string());
}

#[test]
fn managed_consume_known_rollback_stays_available_and_failed_rollback_blocks() {
    use research_agent_core::{
        context::checkpoint_store::CheckpointStoreError, model::ContextPersistenceDiagnostic,
    };
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("data.sqlite")).unwrap();
    let saved = super::super::tests::fixture(&mut sql);
    crate::storage::migrate(&mut sql).unwrap();
    install(&mut sql, &saved).unwrap();
    sql.execute_batch("create trigger fixture_fail before delete on agent_compact_session_start begin select raise(abort,'synthetic failure'); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    {
        let mut guard = db.lock().unwrap();
        assert!(matches!(
            take_managed(&mut guard, saved.conversation_id),
            Err(CheckpointStoreError::Unavailable)
        ));
    }
    {
        let mut guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
        assert!(
            matches!(take_managed(&mut guard, saved.conversation_id), Err(CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "rollback", safe_cause: "sql_rollback_failed", ..
        })) if conversation_id == saved.conversation_id)
        );
    }
    assert!(db.lock().is_err());
}

fn fixture(db: &mut Connection) -> ContextCheckpoint {
    let saved = super::super::tests::fixture(db);
    db.execute_batch(include_str!(
        "../../../../../migrations/0131_agent_compact_session_start.sql"
    ))
    .unwrap();
    db.execute_batch(include_str!(
        "../../../../../migrations/0221_agent_compact_session_start_queue.sql"
    ))
    .unwrap();
    saved
}

#[test]
fn pending_session_start_survives_reopen_and_install_replay_does_not_requeue() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hooks.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    assert_eq!(
        take(&mut db, saved.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        saved.window_id
    );
    assert!(take(&mut db, saved.conversation_id).unwrap().is_none());
    assert_eq!(
        install(&mut db, &saved).unwrap(),
        Installation::AlreadyInstalled { active: true }
    );
    assert!(take(&mut db, saved.conversation_id).unwrap().is_none());
}

#[test]
fn successive_windows_queue_in_order_and_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let first = fixture(&mut db);
    install(&mut db, &first).unwrap();
    let mut second = first.clone();
    second.window_id = Uuid::new_v4();
    second.previous_window_id = Some(first.window_id);
    install(&mut db, &second).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    assert_eq!(
        take(&mut db, first.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        first.window_id
    );
    assert_eq!(
        take(&mut db, first.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        second.window_id
    );
    assert!(take(&mut db, first.conversation_id).unwrap().is_none());
    assert_eq!(
        install(&mut db, &second).unwrap(),
        Installation::AlreadyInstalled { active: true }
    );
    assert!(take(&mut db, first.conversation_id).unwrap().is_none());
}

#[test]
fn pending_from_a_discarded_descendant_is_not_dispatched() {
    let mut db = Connection::open_in_memory().unwrap();
    let first = fixture(&mut db);
    install(&mut db, &first).unwrap();
    let mut second = first.clone();
    second.window_id = Uuid::new_v4();
    second.previous_window_id = Some(first.window_id);
    install(&mut db, &second).unwrap();
    db.execute(
        "update agent_active_context_windows set window_id=?1 where conversation_id=?2",
        params![
            first.window_id.to_string(),
            first.conversation_id.to_string()
        ],
    )
    .unwrap();
    assert_eq!(
        take(&mut db, first.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        first.window_id
    );
    assert!(take(&mut db, first.conversation_id).unwrap().is_none());
}

#[test]
fn enqueue_failure_rolls_back_window_and_pointer_in_the_same_transaction() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    db.execute_batch("create trigger fail_hook_enqueue before insert on agent_compact_session_start begin select raise(abort,'test failure'); end;").unwrap();
    assert_eq!(install(&mut db, &saved), Err(Error::Storage));
    assert!(read_active(&db, saved.conversation_id).unwrap().is_none());
    assert!(take(&mut db, saved.conversation_id).unwrap().is_none());
    let count: u64 = db
        .query_row(
            "select count(*) from agent_context_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn stale_window_pending_hook_is_consumed_without_dispatch() {
    let mut db = Connection::open_in_memory().unwrap();
    let saved = fixture(&mut db);
    install(&mut db, &saved).unwrap();
    db.execute(
        "delete from agent_active_context_windows where conversation_id=?1",
        [saved.conversation_id.to_string()],
    )
    .unwrap();
    assert!(take(&mut db, saved.conversation_id).unwrap().is_none());
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compact_session_start",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
