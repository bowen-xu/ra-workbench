use super::*;
use sha2::{Digest, Sha256};

#[test]
fn failed_object_registration_commit_fences_the_managed_data_owner() {
    let root = tempfile::tempdir().unwrap();
    let objects_path = root.path().join("objects");
    std::fs::create_dir_all(&objects_path).unwrap();
    let objects_path = objects_path.canonicalize().unwrap();
    let mut sql = rusqlite::Connection::open(root.path().join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on;
      create table test_staging_parent(id integer primary key);
      create table test_staging_child(parent_id integer references test_staging_parent(id) deferrable initially deferred);
      create trigger test_staging_commit after insert on content_objects begin insert into test_staging_child(parent_id) values (1); end;").unwrap();
    let database =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(database.clone(), objects_path.clone());
    let objects = ProposalObjects::open(&objects_path).unwrap();
    let bytes = b"synthetic object";
    let version = FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable: false,
    };
    assert!(matches!(
        stage(&store, &objects, &version, bytes),
        Err(Error::RecoveryRequired)
    ));
    assert!(
        database.lock().is_err(),
        "a failed commit must fence subsequent admission"
    );
    let observer = rusqlite::Connection::open(root.path().join("fixture.sqlite3")).unwrap();
    let pending: u64 = observer
        .query_row(
            "select count(*) from agent_object_staging where sha256=?1",
            [&version.sha256],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
    drop(observer);
    drop(objects);
    drop(store);
    drop(database);
    let sql = rusqlite::Connection::open(root.path().join("fixture.sqlite3")).unwrap();
    let reopened =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    reopened
        .lock()
        .unwrap()
        .execute_batch("drop trigger test_staging_commit")
        .unwrap();
    let restored = AppStore::for_test_database(reopened, objects_path);
    assert_eq!(recover(&restored).unwrap(), (1, 0));
    assert!(registered(&restored, &version).unwrap().is_some());
}

#[test]
fn failed_object_registration_rollback_fences_the_managed_data_owner() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let root = tempfile::tempdir().unwrap();
    let objects_path = root.path().join("objects");
    std::fs::create_dir_all(&objects_path).unwrap();
    let objects_path = objects_path.canonicalize().unwrap();
    let mut sql = rusqlite::Connection::open(root.path().join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("create trigger test_staging_insert before insert on content_objects begin select raise(abort,'synthetic insert failure'); end;").unwrap();
    let database =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    {
        let guard = database.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    let store = AppStore::for_test_database(database.clone(), objects_path.clone());
    let objects = ProposalObjects::open(&objects_path).unwrap();
    let bytes = b"synthetic object";
    let version = FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable: false,
    };
    assert!(matches!(
        stage(&store, &objects, &version, bytes),
        Err(Error::RecoveryRequired)
    ));
    assert!(
        database.lock().is_err(),
        "a failed rollback must fence subsequent admission"
    );
    let observer = rusqlite::Connection::open(root.path().join("fixture.sqlite3")).unwrap();
    let pending: u64 = observer
        .query_row(
            "select count(*) from agent_object_staging where sha256=?1",
            [&version.sha256],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
}

#[test]
fn known_registration_failure_rolls_back_without_fencing_and_can_recover() {
    let root = tempfile::tempdir().unwrap();
    let objects_path = root.path().join("objects");
    std::fs::create_dir_all(&objects_path).unwrap();
    let objects_path = objects_path.canonicalize().unwrap();
    let mut sql = rusqlite::Connection::open(root.path().join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("create trigger test_staging_insert before insert on content_objects begin select raise(abort,'synthetic insert failure'); end;").unwrap();
    let database =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(database.clone(), objects_path.clone());
    let objects = ProposalObjects::open(&objects_path).unwrap();
    let bytes = b"synthetic object";
    let version = FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable: false,
    };
    assert!(matches!(
        stage(&store, &objects, &version, bytes),
        Err(Error::Storage)
    ));
    database
        .lock()
        .unwrap()
        .execute_batch("drop trigger test_staging_insert")
        .unwrap();
    assert_eq!(recover(&store).unwrap(), (1, 0));
    assert!(registered(&store, &version).unwrap().is_some());
}

#[test]
fn recovery_reports_an_unconfirmed_commit_instead_of_counting_it_as_a_regular_failure() {
    let root = tempfile::tempdir().unwrap();
    let objects_path = root.path().join("objects");
    std::fs::create_dir_all(&objects_path).unwrap();
    let objects_path = objects_path.canonicalize().unwrap();
    let mut sql = rusqlite::Connection::open(root.path().join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on;
      create table test_staging_parent(id integer primary key);
      create table test_staging_child(parent_id integer references test_staging_parent(id) deferrable initially deferred);
      create trigger test_staging_insert before insert on content_objects begin select raise(abort,'synthetic insert failure'); end;").unwrap();
    let database =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(database.clone(), objects_path.clone());
    let objects = ProposalObjects::open(&objects_path).unwrap();
    let bytes = b"synthetic object";
    let version = FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable: false,
    };
    assert!(matches!(
        stage(&store, &objects, &version, bytes),
        Err(Error::Storage)
    ));
    database.lock().unwrap().execute_batch("drop trigger test_staging_insert;
      create trigger test_staging_commit after insert on content_objects begin insert into test_staging_child(parent_id) values (1); end;").unwrap();
    assert!(matches!(recover(&store), Err(Error::RecoveryRequired)));
    assert!(database.lock().is_err());
}
