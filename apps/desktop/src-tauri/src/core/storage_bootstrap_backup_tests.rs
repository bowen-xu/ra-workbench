use super::*;
use crate::data_interface::Database;
use research_data_runtime::RaftLog;
use rusqlite::params;
use uuid::Uuid;

fn sql(root: &Path, epoch: Uuid, state: &str, position: u64) -> Connection {
    let mut sql = Connection::open(root.join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute(
        "insert into data_runtime_state values(1,1,?1,?2,?3)",
        params![epoch.to_string(), position, state],
    )
    .unwrap();
    sql
}

#[test]
fn preparing_zero_backups_with_missing_initialized_or_partial_logs_restore_normally() {
    for mode in ["missing", "initialized", "lock-created"] {
        let root = tempfile::tempdir().unwrap();
        let epoch = Uuid::new_v4();
        let source = sql(root.path(), epoch, "preparing", 0);
        let path = root.path().join("data-log");
        if mode == "initialized" {
            drop(RaftLog::open(&path, Some(epoch)).unwrap());
        } else if mode == "lock-created" {
            // Published lock_dir creates this ordinary empty control file
            // before queue/runtime metadata initialization. No log codec here.
            fs::create_dir(&path).unwrap();
            fs::File::create(path.join("LOCK")).unwrap();
        }
        let log = Database::lock_existing_pair(&source, &path)
            .unwrap()
            .unwrap();
        assert_eq!(log.head().unwrap().epoch, epoch);
        assert_eq!(log.head().unwrap().accepted_position, 0);
        assert_eq!(
            source
                .query_row("select bootstrap_state from data_runtime_state", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "preparing"
        );
        let future_version = MIGRATIONS.last().unwrap().0 + 1;
        // Only exercise the existing backup selector; no invented migration is
        // registered or applied. Real 207→next-DDL validation remains separate.
        let backup = backup_before_version(
            &source,
            &root.path().join("backups"),
            Some(&log),
            future_version,
        )
        .unwrap()
        .unwrap();
        let backup_sql = Connection::open(&backup).unwrap();
        assert_eq!(
            backup_sql
                .query_row("select bootstrap_state from data_runtime_state", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "preparing"
        );
        drop(backup_sql);
        assert!(Database::lock_existing_pair(&source, &path).is_err());
        let restored = Database::open(
            Connection::open(&backup).unwrap(),
            &backup.with_extension("raft"),
        )
        .unwrap();
        assert_eq!(
            restored
                .lock()
                .unwrap()
                .query_row("select bootstrap_state from data_runtime_state", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "ready"
        );
        assert_eq!(
            restored
                .lock()
                .unwrap()
                .query_row("select epoch from data_runtime_state", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            epoch.to_string()
        );
        restored.shutdown().unwrap();
        let source = Database::open_with_log(source, &path, Some(log)).unwrap();
        assert_eq!(
            source
                .lock()
                .unwrap()
                .query_row("select bootstrap_state from data_runtime_state", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "ready"
        );
        source.shutdown().unwrap();
    }
}

#[test]
fn ready_zero_without_log_and_invalid_preparing_qualifications_never_create_replacements() {
    for (state, position, epoch) in [
        ("ready", 0, Uuid::new_v4()),
        ("preparing", 1, Uuid::new_v4()),
        ("preparing", 0, Uuid::nil()),
    ] {
        let root = tempfile::tempdir().unwrap();
        let source = sql(root.path(), epoch, state, position);
        let path = root.path().join("data-log");
        assert!(Database::lock_existing_pair(&source, &path).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn preparing_with_business_records_is_rejected_by_both_startup_and_backup() {
    let root = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    let source = sql(root.path(), epoch, "preparing", 0);
    let path = root.path().join("data-log");
    let mut log = RaftLog::open(&path, Some(epoch)).unwrap();
    log.append(1, b"must-not-replay-or-replace", false).unwrap();
    let future_version = MIGRATIONS.last().unwrap().0 + 1;
    assert!(
        backup_before_version(
            &source,
            &root.path().join("backups"),
            Some(&log),
            future_version
        )
        .is_err()
    );
    drop(log);
    assert!(Database::lock_existing_pair(&source, &path).is_err());
    let mut log = RaftLog::open(&path, Some(epoch)).unwrap();
    assert_eq!(
        log.read_range(1, 1, 100).unwrap()[0].payload,
        b"must-not-replay-or-replace"
    );
}
