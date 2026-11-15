use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::{
    storage::{StorageError, migrate},
    storage_migration_tests::migrate_through,
};

#[test]
fn repair_migration_63_keeps_its_canonical_checksum_stable() {
    let sql = include_str!("../../migrations/0063_repair_ai_candidate_summaries.sql");
    let checksum = format!("{:x}", Sha256::digest(sql.as_bytes()));

    assert_eq!(
        checksum,
        "003aa681fdf1c0e44d4bd93b201c5a908c378a69dc8677355d4f05c2208e4551"
    );
}

#[test]
fn accepts_migration_63_checksum_recorded_before_terminal_newline_normalization() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate(&mut connection).expect("initial migration");
    connection
        .execute(
            "update migration_checksums set sha256 = ?1 where version = 63",
            ["279d69137ddccd449e801429f99462cbff2da17c812c44b972016564d0219d05"],
        )
        .expect("reproduce recorded development checksum");

    migrate(&mut connection).expect("accept terminal newline normalization");
}

#[test]
fn repairs_force_delete_triggers_after_an_older_110_checksum_was_recorded() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 110);
    connection
        .execute(
            "insert into migration_checksums(version, sha256, recorded_at)
             values (110, ?1, 'now')",
            ["23ab9589f76abe3f71a633c498a24f045dc1ffd6ebb687ffd5111af12d49790e"],
        )
        .expect("older checksum fixture");

    migrate(&mut connection).expect("repair force-delete triggers");

    assert_eq!(
        connection
            .query_row("select max(version) from schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("latest migration"),
        crate::storage_migrations::MIGRATIONS.last().unwrap().0
    );
}

#[test]
fn repairs_registered_migration_49_when_summary_table_is_missing() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 62);
    connection
        .execute_batch("drop table ai_candidate_summary_versions;")
        .expect("reproduce schema drift");

    migrate(&mut connection).expect("repair schema drift");

    assert_eq!(
        schema_object_count(&connection, "table", "ai_candidate_summary_versions"),
        1
    );
    assert_eq!(
        schema_object_count(
            &connection,
            "index",
            "ai_candidate_summary_versions_selected_idx"
        ),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "select count(*) from schema_migrations where version in (49, 63, 64)",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("migration versions"),
        3
    );
    let checksum_versions = connection
        .prepare("select version from migration_checksums order by version")
        .expect("migration checksum query")
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("migration checksum rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("migration checksum versions");
    assert_eq!(
        checksum_versions,
        crate::storage_migrations::MIGRATIONS
            .iter()
            .map(|(version, _)| *version)
            .collect::<Vec<_>>()
    );
    assert_database_integrity(&connection);
}

#[test]
fn repair_migration_preserves_existing_summary_versions() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 62);
    insert_candidate_summary(&connection);

    migrate(&mut connection).expect("apply repair migration");

    let summary: (String, i64) = connection
        .query_row(
            "select summary_text, is_selected from ai_candidate_summary_versions
             where id = 'summary-preserved'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("preserved summary");
    assert_eq!(summary, ("preserve this summary".to_owned(), 1));
    assert_database_integrity(&connection);
}

#[test]
fn upgrades_a_real_version_54_schema_to_the_current_version() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 54);

    migrate(&mut connection).expect("upgrade version 54 schema");

    assert_eq!(
        connection
            .query_row("select max(version) from schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("latest version"),
        crate::storage_migrations::MIGRATIONS.last().unwrap().0
    );
    assert_eq!(
        schema_object_count(&connection, "table", "paper_analysis_workflows"),
        1
    );
    assert_eq!(
        schema_object_count(&connection, "table", "ai_candidate_summary_versions"),
        1
    );
    assert_database_integrity(&connection);
}

#[test]
fn provider_constraint_repair_preserves_historical_openai_api_rows() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 62);
    connection
        .execute_batch(
            "pragma ignore_check_constraints = on;
             insert into ai_provider_settings
             (provider_id, provider_kind, enabled, base_url, region, model_id, thinking_mode,
              timeout_ms, updated_at)
             values ('openai_api', 'generation', 0, '', '', '', 'disabled', 120000, 'now');
             pragma ignore_check_constraints = off;",
        )
        .expect("historical provider row");

    migrate(&mut connection).expect("repair provider constraint");

    assert_eq!(
        connection
            .query_row(
                "select count(*) from ai_provider_settings where provider_id = 'openai_api'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("provider row"),
        1
    );
    assert_database_integrity(&connection);
}

#[test]
fn refuses_a_migration_that_changed_after_its_checksum_was_recorded() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate(&mut connection).expect("migrations");
    connection
        .execute(
            "update migration_checksums set sha256 = ?2 where version = ?1",
            params![49, "0".repeat(64)],
        )
        .expect("corrupt checksum fixture");

    let error = migrate(&mut connection).expect_err("checksum mismatch");
    assert!(matches!(error, StorageError::MigrationIntegrity(_)));
    assert!(error.to_string().contains("迁移 49"));
}

#[test]
fn refuses_to_start_when_a_critical_summary_index_disappears() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate(&mut connection).expect("migrations");
    connection
        .execute_batch("drop index ai_candidate_summary_versions_candidate_idx;")
        .expect("drop critical index");

    let error = migrate(&mut connection).expect_err("schema integrity failure");
    assert!(matches!(error, StorageError::MigrationIntegrity(_)));
    assert!(error.to_string().contains("关键索引"));
}

fn schema_object_count(connection: &Connection, kind: &str, name: &str) -> i64 {
    connection
        .query_row(
            "select count(*) from sqlite_master where type = ?1 and name = ?2",
            params![kind, name],
            |row| row.get(0),
        )
        .expect("schema object count")
}

fn assert_database_integrity(connection: &Connection) {
    let quick_check: String = connection
        .query_row("pragma quick_check", [], |row| row.get(0))
        .expect("quick check");
    assert_eq!(quick_check, "ok");
    let foreign_key_errors = connection
        .prepare("pragma foreign_key_check")
        .expect("foreign key check")
        .query_map([], |_| Ok(()))
        .expect("foreign key rows")
        .count();
    assert_eq!(foreign_key_errors, 0);
}

fn insert_candidate_summary(connection: &Connection) {
    connection
        .execute(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
              output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
              max_output_tokens, timeout_ms, created_at, updated_at)
             values ('run-preserved', 'selection_explanation', 'needs_review',
                     'fixture-generation', 'fixture-v1', 'pipeline', 'prompt', 1, '', ?1,
                     0, 1000, 1000, 5000, 'now', 'now')",
            ["a".repeat(64)],
        )
        .expect("run");
    connection
        .execute(
            "insert into ai_evidence_bundles
             (id, run_id, scope_hash, retrieval_config_version, validation_status, created_at)
             values ('bundle-preserved', 'run-preserved', ?1, 'fixture', 'valid', 'now')",
            ["a".repeat(64)],
        )
        .expect("bundle");
    connection
        .execute(
            "insert into ai_candidates
             (id, run_id, evidence_bundle_id, candidate_kind, schema_version, current_revision,
              review_state, validation_status, original_payload_hash, created_at, updated_at)
             values ('candidate-preserved', 'run-preserved', 'bundle-preserved',
                     'selection_explanation_v1', 1, 1, 'unreviewed', 'valid', ?1, 'now', 'now')",
            ["b".repeat(64)],
        )
        .expect("candidate");
    connection
        .execute(
            "insert into ai_candidate_revisions
             (id, candidate_id, revision, payload_json, payload_hash, editor_kind,
              source_run_id, parent_revision, change_reason, created_at)
             values ('revision-preserved', 'candidate-preserved', 1, '{}', ?1, 'ai',
                     'run-preserved', null, 'provider_generation', 'now')",
            ["b".repeat(64)],
        )
        .expect("revision");
    connection
        .execute(
            "insert into ai_candidate_summary_versions
             (id, candidate_id, candidate_revision, version, summary_text, source_kind,
              is_selected, idempotency_key, created_at)
             values ('summary-preserved', 'candidate-preserved', 1, 1,
                     'preserve this summary', 'human', 1, 'preserve', 'now')",
            [],
        )
        .expect("summary");
}
