//! Compatibility steering writes must retain the data owner on uncertain SQL outcomes.
use super::*;
use crate::data_interface::Database;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn fixture(path: &std::path::Path, log: &std::path::Path) -> (Arc<Database>, SteeringScope) {
    let (db, scope) = setup(Connection::open(path).unwrap());
    let mut sql = Arc::try_unwrap(db).ok().unwrap().into_inner().unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on").unwrap();
    (Database::open(sql, log).unwrap(), scope)
}

#[tokio::test]
async fn stop_fence_serializes_with_admission_and_preserves_the_winning_input_on_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let log = root.path().join("log");
    let (db, scope) = fixture(&path, &log);
    let store = SqliteSteeringStore::new(db.clone());
    let first = input(scope, "before stop");
    let racing = input(scope, "concurrent input");
    store.enqueue(first.clone()).await.unwrap();
    let (seal, admission) = tokio::join!(store.seal_for_stop(scope), store.enqueue(racing.clone()));
    seal.unwrap();
    let mut expected = vec![first];
    match admission {
        Ok(()) => expected.push(racing),
        Err(SteeringError::Closed) => {}
        Err(other) => panic!("unexpected admission failure: {other}"),
    }
    assert_eq!(store.pending(scope, false).await.unwrap(), expected);
    assert_eq!(
        store.enqueue(input(scope, "after stop")).await,
        Err(SteeringError::Closed)
    );
    drop(store);
    drop(db);
    let reopened = Database::open(Connection::open(path).unwrap(), &log).unwrap();
    let store = SqliteSteeringStore::new(reopened);
    assert_eq!(store.pending(scope, false).await.unwrap(), expected);
    assert_eq!(
        store.enqueue(input(scope, "after reopen")).await,
        Err(SteeringError::Closed)
    );
}

#[tokio::test]
async fn commit_failure_blocks_owner_and_reopen_keeps_original_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite");
    let log = root.path().join("log");
    let (db, scope) = fixture(&path, &log);
    db.lock().unwrap().execute_batch("create table steer_fault_parent(id integer primary key); create table steer_fault_child(id integer references steer_fault_parent(id) deferrable initially deferred); create trigger steer_fault_commit after insert on agent_steering_inputs begin insert into steer_fault_child values(1); end;").unwrap();
    let request = input(scope, "retained input");
    let store = SqliteSteeringStore::new(db.clone());
    assert_eq!(
        store.enqueue(request.clone()).await,
        Err(SteeringError::Unavailable)
    );
    assert!(db.lock().is_err());
    drop(store);
    drop(db);
    let reopened = Database::open(Connection::open(path).unwrap(), &log).unwrap();
    let guard = reopened.lock().unwrap();
    let count: i64 = guard
        .query_row(
            "select count(*) from agent_steering_inputs where input_id=?1",
            [request.input_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn confirmed_rejection_keeps_owner_available_but_failed_rollback_blocks() {
    let root = tempfile::tempdir().unwrap();
    let (db, scope) = fixture(&root.path().join("data.sqlite"), &root.path().join("log"));
    db.lock().unwrap().execute_batch("create trigger steer_fault_insert before insert on agent_steering_inputs begin select raise(abort,'synthetic failure'); end;").unwrap();
    let store = SqliteSteeringStore::new(db.clone());
    assert_eq!(
        store.enqueue(input(scope, "first")).await,
        Err(SteeringError::Unavailable)
    );
    {
        let guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    assert_eq!(
        store.enqueue(input(scope, "second")).await,
        Err(SteeringError::Unavailable)
    );
    assert!(db.lock().is_err());
}
