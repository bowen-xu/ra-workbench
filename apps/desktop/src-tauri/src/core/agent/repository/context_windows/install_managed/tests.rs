use super::*;

#[test]
fn commit_failure_blocks_owner_and_reopen_preserves_preinstallation_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let log = root.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    let checkpoint = super::super::tests::fixture(&mut sql);
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fixture_fail after insert on agent_compact_session_start begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &log).unwrap();
    {
        let mut guard = db.lock().unwrap();
        assert!(
            matches!(install(&mut guard, &checkpoint, 1, None), Err(CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "commit", safe_cause: "sql_commit_outcome_unknown", sqlite_extended_code: Some(787),
        })) if conversation_id == checkpoint.conversation_id)
        );
    }
    assert!(db.lock().is_err());
    drop(db);
    let sql = Connection::open(&path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log).unwrap();
    let guard = reopened.lock().unwrap();
    for table in [
        "agent_context_checkpoints",
        "agent_active_context_windows",
        "agent_compact_session_start",
    ] {
        let count: u64 = guard
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn canceled_or_exact_replay_keeps_owner_available() {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("data.sqlite")).unwrap();
    let checkpoint = super::super::tests::fixture(&mut sql);
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let cancellation = AgentCancellation::default();
    cancellation.cancel();
    {
        let mut guard = db.lock().unwrap();
        assert_eq!(
            install(&mut guard, &checkpoint, 1, Some(&cancellation)),
            Err(CheckpointStoreError::Canceled)
        );
    }
    {
        let mut guard = db.lock().unwrap();
        assert_eq!(
            install(&mut guard, &checkpoint, 1, None),
            Ok(Installation::Installed)
        );
        assert_eq!(
            install(&mut guard, &checkpoint, 1, None),
            Ok(Installation::AlreadyInstalled { active: true })
        );
    }
    assert!(db.lock().is_ok());
}

#[test]
fn canceled_install_with_unconfirmed_rollback_fences_owner() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("data.sqlite")).unwrap();
    let checkpoint = super::super::tests::fixture(&mut sql);
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let cancellation = AgentCancellation::default();
    cancellation.cancel();
    {
        let mut guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
        assert!(
            matches!(install(&mut guard, &checkpoint, 1, Some(&cancellation)), Err(CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "rollback", safe_cause: "sql_transaction_remains_open", ..
        })) if conversation_id == checkpoint.conversation_id)
        );
    }
    assert!(db.lock().is_err());
}

#[test]
fn failed_install_releases_owner_only_after_confirmed_rollback() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("data.sqlite")).unwrap();
    let checkpoint = super::super::tests::fixture(&mut sql);
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("create trigger fixture_fail before insert on agent_compact_session_start begin select raise(abort,'synthetic error'); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    {
        let mut guard = db.lock().unwrap();
        assert_eq!(
            install(&mut guard, &checkpoint, 1, None),
            Err(CheckpointStoreError::Unavailable)
        );
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
            matches!(install(&mut guard, &checkpoint, 1, None), Err(CheckpointStoreError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
            conversation_id, phase: "rollback", safe_cause: "sql_rollback_failed", ..
        })) if conversation_id == checkpoint.conversation_id)
        );
    }
    assert!(db.lock().is_err());
}
