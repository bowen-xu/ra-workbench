//! Validate the deployed schema before replay, then retain one log lock across
//! old-schema materialization and migration. No alternative replay lives here.
use super::{StorageError, schema_compat};
use crate::{
    data_interface::Database, storage_integrity::verify_recorded_migration_checksum,
    storage_migrations::MIGRATIONS,
};
use research_data_runtime::RaftLog;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

// The paired data-runtime format first became available in deployed migration 207.
const FIRST_PAIRED_SCHEMA: i64 = 207;
#[path = "storage_startup_terminal_queue.rs"]
mod terminal_queue;

pub(super) fn prepare(
    mut connection: Connection,
    log_path: &Path,
    locked_log: Option<RaftLog>,
) -> Result<(Connection, Option<RaftLog>), StorageError> {
    let versions = applied_versions(&connection)?;
    validate_known_versions(&versions)?;
    let Some(log) = locked_log else {
        // Old SQLite-only databases retain their existing upgrade/checksum
        // compatibility path. There is no accepted log suffix to replay.
        return Ok((connection, None));
    };
    validate_paired_schema(&connection, &versions)?;
    crate::notes::register_projection_functions(&connection)?;
    schema_compat::register_functions(&connection)?;
    crate::data_interface::operation_identity::register(&connection)?;
    connection.execute_batch(
        "pragma foreign_keys = on; pragma journal_mode = wal; pragma synchronous = full;",
    )?;
    terminal_queue::prepare(&mut connection, &versions)?;
    let (connection, log) = Database::prepare_schema_migration(connection, log_path, log)?;
    Ok((connection, Some(log)))
}

fn applied_versions(connection: &Connection) -> Result<Vec<i64>, StorageError> {
    if !table_exists(connection, "schema_migrations")? {
        return Ok(Vec::new());
    }
    let mut query = connection.prepare("select version from schema_migrations order by version")?;
    Ok(query
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn validate_known_versions(versions: &[i64]) -> Result<(), StorageError> {
    validate_known_versions_against(versions, MIGRATIONS)
}

fn validate_known_versions_against(
    versions: &[i64],
    known_migrations: &[(i64, &str)],
) -> Result<(), StorageError> {
    for version in versions {
        if !known_migrations.iter().any(|(known, _)| known == version) {
            return Err(integrity(format!(
                "DATA_RUNTIME_MIGRATION_UNKNOWN_VERSION: version={version}"
            )));
        }
    }
    Ok(())
}

fn validate_paired_schema(connection: &Connection, versions: &[i64]) -> Result<(), StorageError> {
    let known: Vec<_> = MIGRATIONS.iter().map(|(version, _)| *version).collect();
    if versions.last().copied().unwrap_or_default() < FIRST_PAIRED_SCHEMA
        || known.get(..versions.len()) != Some(versions)
    {
        return Err(integrity(
            "DATA_RUNTIME_MIGRATION_INCOMPLETE_PREFIX: paired schema requires a known prefix through 207"
                .into(),
        ));
    }
    if !table_exists(connection, "migration_checksums")? {
        return Err(integrity(
            "DATA_RUNTIME_MIGRATION_CHECKSUMS_MISSING: paired schema requires recorded checksums"
                .into(),
        ));
    }
    for &(version, statements) in MIGRATIONS.iter().take(versions.len()) {
        let recorded: Option<String> = connection
            .query_row(
                "select sha256 from migration_checksums where version=?1",
                [version],
                |row| row.get(0),
            )
            .optional()?;
        let Some(recorded) = recorded else {
            return Err(integrity(format!(
                "DATA_RUNTIME_MIGRATION_CHECKSUM_MISSING: version={version}"
            )));
        };
        if recorded.len() != 64 || !recorded.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(integrity(format!(
                "DATA_RUNTIME_MIGRATION_CHECKSUM_INVALID: version={version}"
            )));
        }
        // Reuse the exact already-recorded compatibility exceptions. Preflight
        // never backfills, replaces or normalizes an existing checksum.
        verify_recorded_migration_checksum(connection, version, statements).map_err(|error| {
            integrity(format!("DATA_RUNTIME_MIGRATION_CHECKSUM_MISMATCH: {error}"))
        })?;
    }
    Ok(())
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, StorageError> {
    Ok(connection.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name=?1)",
        [name],
        |row| row.get(0),
    )?)
}

fn integrity(message: String) -> StorageError {
    StorageError::MigrationIntegrity(message)
}

#[cfg(test)]
#[path = "storage_startup_tests.rs"]
mod tests;
