use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, backup::Backup};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::{storage::StorageError, storage_migrations::MIGRATIONS};

#[cfg(test)]
pub(crate) fn backup_before_pending_migrations(
    connection: &Connection,
    backup_directory: &Path,
) -> Result<Option<PathBuf>, StorageError> {
    backup_before_pending_migrations_locked(connection, backup_directory, None)
}

/// Exercise the normal paired backup on the current deployed schema without
/// inventing or applying another migration, or changing its recorded prefix.
#[cfg(test)]
pub(crate) fn backup_pair_for_next_schema_test(
    connection: &Connection,
    backup_directory: &Path,
    locked_log: &research_data_runtime::RaftLog,
) -> Result<Option<PathBuf>, StorageError> {
    let next = MIGRATIONS.last().map_or(1, |(version, _)| version + 1);
    backup_before_version(connection, backup_directory, Some(locked_log), next)
}

pub(crate) fn backup_before_pending_migrations_locked(
    connection: &Connection,
    backup_directory: &Path,
    locked_log: Option<&research_data_runtime::RaftLog>,
) -> Result<Option<PathBuf>, StorageError> {
    let latest_version = MIGRATIONS.last().map_or(0, |(version, _)| *version);
    backup_before_version(connection, backup_directory, locked_log, latest_version)
}

fn backup_before_version(
    connection: &Connection,
    backup_directory: &Path,
    locked_log: Option<&research_data_runtime::RaftLog>,
    latest_version: i64,
) -> Result<Option<PathBuf>, StorageError> {
    let has_migration_table = connection.query_row(
        "select exists(
             select 1 from sqlite_master where type = 'table' and name = 'schema_migrations'
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    let current_version = if has_migration_table {
        connection
            .query_row("select max(version) from schema_migrations", [], |row| {
                row.get::<_, Option<i64>>(0)
            })?
            .unwrap_or(0)
    } else {
        0
    };
    if current_version >= latest_version {
        return Ok(None);
    }

    fs::create_dir_all(backup_directory)?;
    #[cfg(unix)]
    fs::set_permissions(backup_directory, fs::Permissions::from_mode(0o700))?;
    let timestamp =
        connection.query_row("select strftime('%Y%m%dT%H%M%fZ', 'now')", [], |row| {
            row.get::<_, String>(0)
        })?;
    let backup_path = backup_directory.join(format!(
        "pre-migration-v{current_version}-to-v{latest_version}-{timestamp}-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let mut destination = Connection::open(&backup_path)?;
    let backup = Backup::new(connection, &mut destination)?;
    backup.run_to_completion(256, Duration::from_millis(5), None)?;
    drop(backup);
    destination.execute_batch("pragma wal_checkpoint(truncate);")?;
    #[cfg(unix)]
    fs::set_permissions(&backup_path, fs::Permissions::from_mode(0o600))?;
    backup_log_pair(&destination, &backup_path, locked_log)?;
    Ok(Some(backup_path))
}

/// The held Engine LOCK excludes other writers/reclamation until the copy is
/// complete. Epoch/watermark are read from the destination SQLite snapshot.
fn backup_log_pair(
    db: &Connection,
    sql_path: &Path,
    locked_log: Option<&research_data_runtime::RaftLog>,
) -> Result<(), StorageError> {
    let paired: bool = db.query_row(
        "select exists(select 1 from sqlite_master where name='data_runtime_state')",
        [],
        |r| r.get(0),
    )?;
    if !paired {
        return Ok(());
    }
    use rusqlite::OptionalExtension;
    let state: Option<(u32,String,u64,String)> = db.query_row("select schema_version,epoch,materialized_position,bootstrap_state from data_runtime_state where singleton=1", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((schema, epoch, position, state)) = state else {
        return Ok(());
    };
    let locked_log = locked_log.ok_or_else(|| {
        StorageError::MigrationIntegrity("paired backup requires exclusive log ownership".into())
    })?;
    let head = locked_log
        .head()
        .map_err(crate::data_interface::DataError::from)?;
    if schema != 1
        || head.epoch.is_nil()
        || head.epoch.to_string() != epoch
        || position > head.accepted_position
        || position < head.compacted_position
    {
        return Err(StorageError::MigrationIntegrity(
            "backup SQL snapshot does not match retained log prefix".into(),
        ));
    }
    let source = locked_log.path();
    let preparing_empty = state == "preparing"
        && position == 0
        && head.accepted_position == 0
        && head.compacted_position == 0;
    if !(state == "ready" || preparing_empty) || !source.is_dir() {
        return Err(StorageError::MigrationIntegrity(
            "paired backup requires a ready log or a proven empty preparing pair".into(),
        ));
    }
    let destination = sql_path.with_extension("raft");
    fs::create_dir(&destination)?;
    #[cfg(unix)]
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(StorageError::MigrationIntegrity(
                "unexpected non-file in Raft backup".into(),
            ));
        }
        let target = destination.join(entry.file_name());
        fs::copy(entry.path(), &target)?;
        #[cfg(unix)]
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
        fs::OpenOptions::new()
            .write(true)
            .open(&target)?
            .sync_all()?;
    }
    #[cfg(unix)]
    fs::File::open(&destination)?.sync_all()?;
    let manifest = serde_json::json!({"schemaVersion":1,"epoch":epoch,"materializedPosition":position,"acceptedPosition":head.accepted_position,
        "sqlite":sql_path.file_name(),"raftDirectory":destination.file_name(),"complete":true});
    let path = sql_path.with_extension("pair.json");
    let pending = sql_path.with_extension("pair.pending");
    fs::write(
        &pending,
        serde_json::to_vec_pretty(&manifest)
            .map_err(|_| StorageError::MigrationIntegrity("backup manifest encoding".into()))?,
    )?;
    #[cfg(unix)]
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))?;
    fs::OpenOptions::new()
        .write(true)
        .open(&pending)?
        .sync_all()?;
    fs::rename(pending, &path)?;
    #[cfg(unix)]
    fs::File::open(
        sql_path
            .parent()
            .ok_or_else(|| StorageError::MigrationIntegrity("backup parent missing".into()))?,
    )?
    .sync_all()?;
    Ok(())
}

#[cfg(test)]
#[path = "storage_bootstrap_backup_tests.rs"]
mod bootstrap_tests;

#[cfg(test)]
#[path = "storage_sql_immediate_backup_tests.rs"]
mod sql_immediate_tests;

#[cfg(test)]
#[path = "storage_backup_rename_tests.rs"]
mod rename_tests;

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn creates_a_recoverable_backup_before_pending_migrations() {
        let root = temporary_directory();
        fs::create_dir_all(&root).expect("backup test directory");
        let connection =
            Connection::open(root.join("research-assistant.sqlite3")).expect("source database");
        connection
            .execute_batch(
                "create table schema_migrations (
                     version integer primary key,
                     applied_at text not null
                 );
                 insert into schema_migrations values (1, 'now');
                 create table backup_sentinel (value text not null);
                 insert into backup_sentinel values ('preserved');",
            )
            .expect("legacy fixture");

        let backup_path = backup_before_pending_migrations(&connection, &root.join("backups"))
            .expect("create pre-migration backup")
            .expect("pending migrations require backup");
        let backup = Connection::open(&backup_path).expect("open backup");
        let value = backup
            .query_row("select value from backup_sentinel", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("read backed-up data");
        assert_eq!(value, "preserved");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&backup_path)
                .expect("backup metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        drop(backup);
        drop(connection);
        fs::remove_dir_all(root).expect("cleanup migration backup fixture");
    }

    fn temporary_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("research-assistant-migration-backup-{nonce}"))
    }

    #[test]
    fn paired_backup_requires_log_lock_and_restores_unmaterialized_suffix() {
        use crate::data_interface::{Database, journal::JournalCommand};
        use research_agent_core::conversation::ConversationId;
        use research_data_runtime::coordinator::WriteOptions;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.sqlite3");
        let mut sql = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let database = Database::open(sql, &root.path().join("data-log")).unwrap();
        let id = ConversationId::from_uuid(uuid::Uuid::new_v4());
        database
            .write(
                JournalCommand::new(
                    id,
                    uuid::Uuid::new_v4(),
                    0,
                    vec![],
                    Some("paired backup".into()),
                    1_790_000_000_000,
                )
                .unwrap(),
                WriteOptions::default(),
            )
            .unwrap();
        drop(database);
        let source = Connection::open(&path).unwrap();
        let future_version = MIGRATIONS.last().unwrap().0 + 1; // Exercise a later migration gate, without inventing a migration.
        assert!(
            backup_before_version(&source, &root.path().join("unowned"), None, future_version)
                .is_err()
        );
        let log = Database::lock_existing_pair(&source, &root.path().join("data-log"))
            .unwrap()
            .unwrap();
        let backup = backup_before_version(
            &source,
            &root.path().join("arbitrary").join("backups"),
            Some(&log),
            future_version,
        )
        .unwrap()
        .unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(backup.with_extension("pair.json")).unwrap()).unwrap();
        assert_eq!(manifest["materializedPosition"], 0);
        assert_eq!(manifest["acceptedPosition"], 1);
        let restored = Database::open(
            Connection::open(&backup).unwrap(),
            &backup.with_extension("raft"),
        )
        .unwrap();
        assert_eq!(
            restored
                .lock()
                .unwrap()
                .query_row("select count(*) from agent_conversations", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            1
        );
        // Acquiring another owner while the source is held must fail, including
        // before a migration/backup starts mutating either storage half.
        assert!(Database::lock_existing_pair(&source, &root.path().join("data-log")).is_err());
    }
}
