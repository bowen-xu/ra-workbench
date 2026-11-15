use std::collections::BTreeSet;

use super::*;

#[test]
fn migrations_are_ordered_and_idempotent() {
    let mut connection = Connection::open_in_memory().expect("open database");
    migrate(&mut connection).expect("first migration");
    migrate(&mut connection).expect("second migration");

    let versions = connection
        .prepare("select version from schema_migrations order by version")
        .expect("prepare versions")
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("read versions")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect versions");
    let expected = MIGRATIONS
        .iter()
        .map(|(version, _)| *version)
        .collect::<Vec<_>>();
    assert_eq!(versions, expected);
}

#[test]
fn research_knowledge_migration_preserves_legacy_claims_and_evidence_links() {
    let connection = Connection::open_in_memory().expect("open database");
    connection
        .execute_batch(
            "pragma foreign_keys = on;
             create table claims (
                 id text primary key, project_id text, statement text not null,
                 origin text not null, created_at text not null, updated_at text not null
             );
             create table evidence_links (
                 id text primary key, claim_id text not null, anchor_id text not null,
                 stance text not null, rationale text not null, created_at text not null
             );
             insert into claims values
                 ('human', null, 'Human claim', 'user', 'now', 'now'),
                 ('legacy', null, 'Legacy AI claim', 'ai', 'now', 'now');
             insert into evidence_links values
                 ('link', 'human', 'anchor', 'supports', 'direct quote', 'now');",
        )
        .expect("legacy schema fixture");
    connection
        .execute_batch(include_str!("../../migrations/0017_research_knowledge.sql"))
        .expect("apply research knowledge migration");

    let migrated = connection
        .prepare("select id, provenance_kind, revision from claims order by id")
        .expect("prepare migrated claims")
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .expect("read migrated claims")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect migrated claims");
    assert_eq!(
        migrated,
        vec![
            ("human".to_owned(), "human_original".to_owned(), 1),
            ("legacy".to_owned(), "legacy_ai".to_owned(), 1),
        ]
    );
    assert_eq!(
        connection
            .query_row("select count(*) from evidence_links", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "select count(*) from research_object_revisions",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn unified_content_reference_view_covers_every_owning_foreign_key() {
    let mut connection = Connection::open_in_memory().expect("open database");
    migrate(&mut connection).expect("migrate database");
    let tables = connection
        .prepare("select name from sqlite_master where type = 'table' order by name")
        .expect("prepare tables")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("read tables")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect tables");
    let mut owning_tables = BTreeSet::new();
    for table in tables {
        let escaped = table.replace('"', "\"\"");
        let sql = format!("pragma foreign_key_list(\"{escaped}\")");
        let mut statement = connection.prepare(&sql).expect("foreign keys");
        for foreign_key in statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            })
            .expect("read foreign keys")
        {
            let (target_table, source_column) = foreign_key.expect("foreign key");
            if target_table == "content_objects"
                && source_column == "content_sha256"
                && table != "content_object_gc_candidates"
            {
                owning_tables.insert(table.clone());
            }
        }
    }
    assert_eq!(
        owning_tables,
        BTreeSet::from([
            "agent_evidence_images".to_owned(),
            "agent_hook_spill_artifacts".to_owned(),
            "agent_note_patch_assets".to_owned(),
            "attachments".to_owned(),
            "docling_parse_runs".to_owned(),
            "hbs_note_baseline_assets".to_owned(),
            "hbs_preservation_assets".to_owned(),
            "manuscript_compile_artifacts".to_owned(),
            "note_block_assets".to_owned(),
            "note_revision_assets".to_owned(),
            "source_anchors".to_owned(),
        ])
    );

    let view_sql: String = connection
        .query_row(
            "select sql from sqlite_master where type = 'view' and name = 'content_object_references'",
            [],
            |row| row.get(0),
        )
        .expect("reference view");
    for table in owning_tables {
        assert!(
            view_sql.contains(&format!("from {table}")),
            "content reference view does not include {table}"
        );
    }
}

#[test]
fn marks_unfinished_jobs_as_interrupted_on_restart() {
    let mut connection = Connection::open_in_memory().expect("open database");
    migrate(&mut connection).expect("migrate database");
    connection
        .execute(
            "insert into import_jobs (id, source_kind, state, created_at, updated_at)
             values ('job', 'pdf', 'running', 'now', 'now')",
            [],
        )
        .expect("insert running job");
    connection
        .execute(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version,
              prompt_version, output_schema_version, user_goal, scope_hash,
              input_item_count, max_input_tokens, max_output_tokens, timeout_ms,
              created_at, updated_at)
             values ('run', 'provider_diagnostic', 'generating', 'fixture-generation',
                     'fixture-v1', 'runtime-v1', 'diagnostic-v1', 1, '', ?1,
                     0, 100, 100, 5000, 'now', 'now')",
            ["a".repeat(64)],
        )
        .expect("insert running AI run");
    connection
        .execute(
            "insert into ai_run_stages
             (id, run_id, stage_kind, attempt, status, created_at)
             values ('stage', 'run', 'generating', 1, 'running', 'now')",
            [],
        )
        .expect("insert running AI stage");
    migrate(&mut connection).expect("simulate restart");
    let state: String = connection
        .query_row(
            "select state from import_jobs where id = 'job'",
            [],
            |row| row.get(0),
        )
        .expect("read state");
    assert_eq!(state, "interrupted");
    let (run_status, run_error): (String, String) = connection
        .query_row(
            "select status, error_code from ai_runs where id = 'run'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read recovered AI run");
    assert_eq!(
        (run_status.as_str(), run_error.as_str()),
        ("failed", "interrupted")
    );
    let stage_status: String = connection
        .query_row(
            "select status from ai_run_stages where id = 'stage'",
            [],
            |row| row.get(0),
        )
        .expect("read recovered AI stage");
    assert_eq!(stage_status, "failed");
}

#[test]
fn marks_shared_paper_submission_terminal_state_on_restart() {
    let mut connection = Connection::open_in_memory().expect("open database");
    migrate(&mut connection).expect("migrate database");
    connection
        .execute_batch("pragma foreign_keys = off;")
        .expect("disable foreign keys for isolated fixture");
    connection
        .execute(
            "insert into paper_analysis_turn_attempts
             (id, turn_id, attempt, status, normalized_messages_json,
              provider_id, model_id, created_at)
             values ('attempt', 'turn', 1, 'running', '[]', 'missing-provider', 'fixture', 'now')",
            [],
        )
        .expect("insert running paper attempt");
    connection
        .execute(
            "insert into agent_paper_analysis_submissions
             (attempt_id, conversation_id, request_id, turn_id,
              context_snapshot_id, paper_id, created_at)
             values ('attempt', 'conversation', 'request', 'shared-turn',
                     'snapshot', 'paper', 'now')",
            [],
        )
        .expect("insert shared submission");
    connection
        .execute_batch("pragma foreign_keys = on;")
        .expect("restore foreign keys");

    migrate(&mut connection).expect("simulate restart");
    let terminal_status: String = connection
        .query_row(
            "select terminal_status from agent_paper_analysis_submissions where attempt_id = 'attempt'",
            [],
            |row| row.get(0),
        )
        .expect("read shared terminal status");
    assert_eq!(terminal_status, "interrupted");
}
