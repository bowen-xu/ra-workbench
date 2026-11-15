//! One numbered compatibility edge before an unpublished owner drains its log.
//! The factory has already backed up the locked pair; no business facts are
//! rewritten and the ordinary projector remains the only accepted-log replayer.
use super::*;

const DEPLOYED_SCHEMA: i64 = 213;
const COMPATIBILITY_SCHEMA: i64 = 215;
const TRIGGER: &str = "agent_terminal_queue_pause";

pub(super) fn prepare(connection: &mut Connection, versions: &[i64]) -> Result<(), StorageError> {
    if versions.last() != Some(&DEPLOYED_SCHEMA) {
        // Older paired schemas retain their ordinary old-schema drain contract.
        // In particular, this does not skip outstanding 208–213 migrations.
        return Ok(());
    }
    let Some(&(version, statements)) = MIGRATIONS.get(versions.len()) else {
        return Ok(());
    };
    if version != COMPATIBILITY_SCHEMA {
        return Err(integrity(
            "DATA_RUNTIME_MIGRATION_COMPATIBILITY_EDGE_CHANGED: expected 213 to 215".into(),
        ));
    }
    let actual: Option<String> = connection
        .query_row(
            "select sql from sqlite_schema where type='trigger' and name=?1 and tbl_name='agent_journal_records'",
            [TRIGGER],
            |row| row.get(0),
        )
        .optional()?;
    if actual.as_deref() != Some(deployed_trigger()?.as_str()) {
        return Err(integrity(
            "DATA_RUNTIME_MIGRATION_TERMINAL_TRIGGER_MISMATCH: deployed 213 trigger changed".into(),
        ));
    }
    // This is the registered 215 migration, including its version and checksum
    // in the existing transaction. A crash after commit is recognizable as 215;
    // reopening verifies that checksum and drains without executing this DDL again.
    super::super::apply_migration(connection, version, statements)
}

fn deployed_trigger() -> Result<String, StorageError> {
    let statements = MIGRATIONS
        .iter()
        .find(|(version, _)| *version == 94)
        .ok_or_else(|| integrity("DATA_RUNTIME_MIGRATION_TERMINAL_SOURCE_MISSING".into()))?
        .1;
    let prefix = "create trigger agent_terminal_queue_pause";
    let (_, body) = statements
        .split_once(prefix)
        .ok_or_else(|| integrity("DATA_RUNTIME_MIGRATION_TERMINAL_SOURCE_INVALID".into()))?;
    // Let the bundled engine reproduce its own schema normalization and the
    // deployed 175 table rename. Comparing exact SQL avoids accepting altered
    // guards through a permissive whitespace/string normalization rule.
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(
        "create table agent_items(conversation_id text,payload_json text);
         create table agent_input_queues(conversation_id text primary key,revision integer,snapshot_json text);",
    )?;
    reference.execute_batch(&format!("{prefix}{body}"))?;
    reference.execute_batch("alter table agent_items rename to agent_journal_records;")?;
    Ok(reference.query_row(
        "select sql from sqlite_schema where type='trigger' and name=?1",
        [TRIGGER],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
#[path = "storage_startup_terminal_queue_tests.rs"]
mod tests;
