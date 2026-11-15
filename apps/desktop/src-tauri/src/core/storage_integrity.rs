use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{storage::StorageError, storage_migrations::MIGRATIONS};

const MIGRATION_63_RECORDED_WITH_TRAILING_BLANK_LINE: &str =
    "279d69137ddccd449e801429f99462cbff2da17c812c44b972016564d0219d05";
const MIGRATION_63_CANONICAL_CHECKSUM: &str =
    "003aa681fdf1c0e44d4bd93b201c5a908c378a69dc8677355d4f05c2208e4551";
const MIGRATION_110_EARLY_FORCE_DELETE_CHECKSUM: &str =
    "23ab9589f76abe3f71a633c498a24f045dc1ffd6ebb687ffd5111af12d49790e";
const MIGRATION_114_ORIGINAL_CHECKSUM: &str =
    "db9b6eea85eea41d6e8658d2bf058980f0827b678815cde191bfe089b366c435";
const MIGRATION_114_HARDENED_IN_PLACE_CHECKSUM: &str =
    "42f00e2d7ebec85129f493ad665cff5e13a6b22cdb4fcfff10fcc97bf9e985f5";
const MIGRATION_118_ORIGINAL_CHECKSUM: &str =
    "6be120040299e7033fc2a6568e7b6e7cfe23d37fbaf173f8f5fcccc2be139278";
const MIGRATION_118_CORRECTED_IN_PLACE_CHECKSUM: &str =
    "e9b9421430411680fdc518034a3cf00c63246bcd7621ec9716cf93520a1932b7";
const MIGRATION_136_ORIGINAL_CHECKSUM: &str =
    "bd3be6e33b9eac03736470114ebf563a55dc8cc698bb64c0c7be038ff4717467";
const MIGRATION_136_HARDENED_IN_PLACE_CHECKSUM: &str =
    "f720122dafd68e14302352941e8987bea4922bf0c4798fbf99a913e12b7be463";
const MIGRATION_143_DEPLOYED_CHECKSUM: &str =
    "4c772a1ce2bccc8ac95feebf4e0437b4518cedf4dc4f53cec03c5738ec8d7482";
const MIGRATION_143_RETENTION_IN_PLACE_CHECKSUM: &str =
    "0ae1becddfb7faa5f226352dc3004de5ebb68692a29b81ca3082912dece2e087";
const MIGRATION_147_DEPLOYED_CHECKSUM: &str =
    "a0b376c0cd3b74b92b35e20563515b151e1376b909fb66f16859c066881ccd0d";
const MIGRATION_147_CLEANUP_IN_PLACE_CHECKSUM: &str =
    "bd009123cb810d7d3bba6365186da07df4652aed80a3a7a29dc7c31a2aeb7077";
const MIGRATION_184_ORIGINAL_CHECKSUM: &str =
    "bb2948706fedca09d9229d43d74772c3911ac3ac7f1ef4cde8c7b24d23573cef";
const MIGRATION_184_PAGE_FK_IN_PLACE_CHECKSUM: &str =
    "7248840b62df4b0e4d6e4ace7eb96f7278f32289d2a4ef0a478548d5ad85fc54";

pub(crate) fn verify_recorded_migration_checksum(
    connection: &Connection,
    version: i64,
    statements: &str,
) -> Result<(), StorageError> {
    if !table_exists(connection, "migration_checksums")? {
        return Ok(());
    }
    let recorded = connection
        .query_row(
            "select sha256 from migration_checksums where version = ?1",
            [version],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let current = migration_checksum(statements);
    if let Some(value) = recorded
        && value != current
        && !is_known_migration_checksum_change(connection, version, &value, &current)
    {
        return Err(StorageError::MigrationIntegrity(format!(
            "迁移 {version} 的内容与首次记录的校验和不一致（数据库记录：{value}；当前构建：{current}）"
        )));
    }
    Ok(())
}

fn is_known_migration_checksum_change(
    connection: &Connection,
    version: i64,
    recorded: &str,
    current: &str,
) -> bool {
    (version == 63
        && recorded == MIGRATION_63_RECORDED_WITH_TRAILING_BLANK_LINE
        && current == MIGRATION_63_CANONICAL_CHECKSUM)
        || (version == 110
            && recorded == MIGRATION_110_EARLY_FORCE_DELETE_CHECKSUM
            && table_exists(connection, "agent_force_delete_markers").unwrap_or(false))
        // v114 was hardened in place during development after the original
        // had been applied. Keep both recorded hashes immutable; migration 115
        // installs the same final guard on either schema, inside a transaction.
        || (version == 114
            && recorded == MIGRATION_114_HARDENED_IN_PLACE_CHECKSUM
            && current == MIGRATION_114_ORIGINAL_CHECKSUM)
        // v118's manuscript join was corrected after the early definition had
        // run. Preserve either exact recorded checksum; v121 repairs the guard
        // transactionally, including databases that already reached v120.
        || (version == 118
            && recorded == MIGRATION_118_CORRECTED_IN_PLACE_CHECKSUM
            && current == MIGRATION_118_ORIGINAL_CHECKSUM)
        // v136's journal boundary FK was added after the early schema ran.
        // v142 rebuilds either exact known definition and checks all FKs in
        // the publication transaction; never rewrite the original journal hash.
        || (version == 136
            && recorded == MIGRATION_136_HARDENED_IN_PLACE_CHECKSUM
            && current == MIGRATION_136_ORIGINAL_CHECKSUM)
        // The early v143 is already deployed; retention was appended before its
        // first Git commit. Freeze the current bytes and preserve the recorded
        // early hash. v144 installs retention transactionally on both schemas.
        || (version == 143
            && recorded == MIGRATION_143_DEPLOYED_CHECKSUM
            && current == MIGRATION_143_RETENTION_IN_PLACE_CHECKSUM)
        // v147 acquired cleanup triggers after the early definition had run.
        // Freeze its committed bytes and preserve both exact recorded hashes;
        // v148 installs the cleanup transactionally on either deployed schema.
        || (version == 147
            && recorded == MIGRATION_147_DEPLOYED_CHECKSUM
            && current == MIGRATION_147_CLEANUP_IN_PLACE_CHECKSUM)
        // Restore v184's exact original bytes. Preserve the in-place variant's
        // recorded hash; v191 upgrades either schema without losing receipts.
        || (version == 184
            && recorded == MIGRATION_184_PAGE_FK_IN_PLACE_CHECKSUM
            && current == MIGRATION_184_ORIGINAL_CHECKSUM)
}

pub(crate) fn record_migration_checksum(
    connection: &Connection,
    version: i64,
    statements: &str,
) -> Result<(), StorageError> {
    if table_exists(connection, "migration_checksums")? {
        connection.execute(
            "insert into migration_checksums (version, sha256, recorded_at)
             values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(version) do nothing",
            params![version, migration_checksum(statements)],
        )?;
    }
    Ok(())
}

pub(crate) fn backfill_migration_checksums(connection: &Connection) -> Result<(), StorageError> {
    for &(version, statements) in MIGRATIONS {
        let was_applied = connection.query_row(
            "select exists(select 1 from schema_migrations where version = ?1)",
            [version],
            |row| row.get::<_, bool>(0),
        )?;
        if was_applied {
            record_migration_checksum(connection, version, statements)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_ai_candidate_schema(connection: &Connection) -> Result<(), StorageError> {
    const TABLE: &str = "ai_candidate_summary_versions";
    if !table_exists(connection, TABLE)? {
        return Err(StorageError::MigrationIntegrity(format!(
            "关键表 {TABLE} 缺失"
        )));
    }

    let columns = connection
        .prepare("pragma table_info(ai_candidate_summary_versions)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<BTreeSet<_>, _>>()?;
    let expected_columns = BTreeSet::from([
        "candidate_id".to_owned(),
        "candidate_revision".to_owned(),
        "created_at".to_owned(),
        "discarded_at".to_owned(),
        "id".to_owned(),
        "idempotency_key".to_owned(),
        "is_selected".to_owned(),
        "source_kind".to_owned(),
        "summary_text".to_owned(),
        "version".to_owned(),
    ]);
    if columns != expected_columns {
        return Err(StorageError::MigrationIntegrity(format!(
            "关键表 {TABLE} 的列定义不完整"
        )));
    }

    for index in [
        "ai_candidate_summary_versions_selected_idx",
        "ai_candidate_summary_versions_candidate_idx",
    ] {
        let exists = connection.query_row(
            "select exists(select 1 from sqlite_master
             where type = 'index' and name = ?1 and tbl_name = ?2)",
            params![index, TABLE],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(StorageError::MigrationIntegrity(format!(
                "关键索引 {index} 缺失"
            )));
        }
    }

    let foreign_keys = connection
        .prepare("pragma foreign_key_list(ai_candidate_summary_versions)")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<BTreeSet<_>, _>>()?;
    let expected_foreign_keys = BTreeSet::from([
        (
            "ai_candidate_revisions".to_owned(),
            "candidate_id".to_owned(),
            "candidate_id".to_owned(),
        ),
        (
            "ai_candidate_revisions".to_owned(),
            "candidate_revision".to_owned(),
            "revision".to_owned(),
        ),
        (
            "ai_candidates".to_owned(),
            "candidate_id".to_owned(),
            "id".to_owned(),
        ),
    ]);
    if foreign_keys != expected_foreign_keys {
        return Err(StorageError::MigrationIntegrity(format!(
            "关键表 {TABLE} 的外键定义不完整"
        )));
    }

    let violation_count = connection
        .prepare("pragma foreign_key_check(ai_candidate_summary_versions)")?
        .query_map([], |_| Ok(()))?
        .count();
    if violation_count > 0 {
        return Err(StorageError::MigrationIntegrity(format!(
            "关键表 {TABLE} 存在 {violation_count} 个外键错误"
        )));
    }
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "select exists(select 1 from sqlite_master where type = 'table' and name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn migration_checksum(statements: &str) -> String {
    format!("{:x}", Sha256::digest(statements.as_bytes()))
}
