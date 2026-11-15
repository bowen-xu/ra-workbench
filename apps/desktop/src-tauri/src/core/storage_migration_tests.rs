use rusqlite::Connection;

use crate::storage_migrations::{MIGRATIONS, MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED};
#[path = "storage_migration_tests/child_cleanup_intents.rs"]
mod child_cleanup_intents;
#[path = "storage_migration_tests/compact_session_queue.rs"]
mod compact_session_queue;
#[path = "storage_migration_tests/deleted_workspace_gc.rs"]
mod deleted_workspace_gc;
#[path = "storage_migration_tests/hook_spill_receipts.rs"]
mod hook_spill_receipts;
#[path = "storage_migration_tests/legacy_event_intent.rs"]
mod legacy_event_intent;
#[path = "storage_migration_tests/legacy_import_recovery_guard.rs"]
mod legacy_import_recovery_guard;
#[path = "storage_migration_tests/note_conflict_receipt_pages.rs"]
mod note_conflict_receipt_pages;
#[path = "storage_migration_tests/paper_generation.rs"]
mod paper_generation;
#[path = "storage_migration_tests/paper_portraits.rs"]
mod paper_portraits;
#[path = "storage_migration_tests/paper_templates.rs"]
mod paper_templates;
#[path = "storage_migration_tests/provider_window_upgrade.rs"]
mod provider_window_upgrade;
#[path = "storage_migration_tests/qwen_provider_compatibility.rs"]
mod qwen_provider_compatibility;
#[path = "storage_migration_tests/reading_source_retention.rs"]
mod reading_source_retention;
#[path = "storage_migration_tests/sql_immediate_receipts.rs"]
mod sql_immediate_receipts;
#[path = "storage_migration_tests/steering_handoff.rs"]
mod steering_handoff;
#[path = "storage_migration_tests/stop_hook_audit.rs"]
mod stop_hook_audit;
#[path = "storage_migration_tests/terminal_queue_versions.rs"]
mod terminal_queue_versions;
#[path = "storage_migration_tests/text_patch_contracts.rs"]
mod text_patch_contracts;
#[path = "storage_migration_tests/writing_delivery_proofs.rs"]
mod writing_delivery_proofs;

pub(crate) fn migrate_through(connection: &mut Connection, maximum_version: i64) {
    crate::data_interface::operation_identity::register(connection)
        .expect("journal operation function");
    crate::notes::register_projection_functions(connection).expect("note projection functions");
    crate::storage::schema_compat::register_functions(connection)
        .expect("writing note projection functions");
    connection
        .execute_batch(
            "pragma foreign_keys = on;
             create table schema_migrations (
                 version integer primary key,
                 applied_at text not null
             );",
        )
        .expect("migration registry");
    for &(version, sql) in MIGRATIONS {
        if version > maximum_version {
            break;
        }
        let requires_foreign_keys_disabled =
            MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED.contains(&version);
        if requires_foreign_keys_disabled {
            connection
                .execute_batch("pragma foreign_keys = off;")
                .expect("disable foreign keys for table rebuild");
        }
        let transaction = connection.transaction().expect("migration transaction");
        transaction.execute_batch(sql).expect("apply migration");
        if version == 212 {
            crate::agent::repository::search_visibility::backfill_in(&transaction)
                .expect("backfill rollback search visibility");
        }
        transaction
            .execute(
                "insert into schema_migrations (version, applied_at) values (?1, 'now')",
                [version],
            )
            .expect("record migration");
        transaction.commit().expect("commit migration");
        if requires_foreign_keys_disabled {
            connection
                .execute_batch("pragma foreign_keys = on;")
                .expect("restore foreign keys after table rebuild");
        }
    }
}

#[test]
fn expanded_literature_types_preserve_existing_relationships() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 42);
    connection
        .execute_batch(
            "insert into research_projects (id, title, summary, created_at)
             values ('project-types', 'Types', '', 'now');
             insert into bibliographic_items
                 (id, item_type, title, created_at, updated_at)
             values ('paper-types', 'journalArticle', 'Existing paper', 'now', 'now');
             insert into project_items (project_id, item_id, added_at)
             values ('project-types', 'paper-types', 'now');
             insert into metadata_provenance
                 (item_id, field_name, source_kind, confirmed_by_user, updated_at)
             values ('paper-types', 'title', 'user', 1, 'now');",
        )
        .expect("seed literature relationships");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 43).then_some(*sql))
        .expect("migration 43");
    connection
        .execute_batch("pragma foreign_keys = off;")
        .expect("disable foreign keys");
    let transaction = connection.transaction().expect("migration transaction");
    transaction
        .execute_batch(migration)
        .expect("expand literature types");
    transaction.commit().expect("commit migration");
    connection
        .execute_batch("pragma foreign_keys = on;")
        .expect("restore foreign keys");

    let relationship_counts: (i64, i64) = connection
        .query_row(
            "select (select count(*) from project_items where item_id = 'paper-types'),
                    (select count(*) from metadata_provenance where item_id = 'paper-types')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("relationship counts");
    assert_eq!(relationship_counts, (1, 1));
    let foreign_key_errors = connection
        .prepare("pragma foreign_key_check")
        .expect("foreign key check")
        .query_map([], |_| Ok(()))
        .expect("foreign key rows")
        .count();
    assert_eq!(foreign_key_errors, 0);

    for (index, item_type) in [
        "magazineArticle",
        "newspaperArticle",
        "book",
        "bookSection",
        "manuscript",
        "thesis",
        "dataset",
        "computerProgram",
        "webpage",
        "standard",
        "patent",
    ]
    .iter()
    .enumerate()
    {
        connection
            .execute(
                "insert into bibliographic_items
                    (id, item_type, title, created_at, updated_at)
                 values (?1, ?2, 'Supported type', 'now', 'now')",
                (format!("supported-type-{index}"), item_type),
            )
            .expect("insert supported literature type");
    }
    assert!(
        connection
            .execute(
                "insert into bibliographic_items
                (id, item_type, title, created_at, updated_at)
             values ('unsupported-type', 'proceedings', 'Unsupported type', 'now', 'now')",
                [],
            )
            .is_err()
    );
}

#[test]
fn conference_metadata_migration_preserves_items_and_adds_empty_fields() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 43);
    connection
        .execute(
            "insert into bibliographic_items
                (id, item_type, title, container_title, created_at, updated_at)
             values ('paper-conference-fields', 'bookSection', 'Existing chapter',
                     'Existing container', 'now', 'now')",
            [],
        )
        .expect("seed existing item");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 44).then_some(*sql))
        .expect("migration 44");
    connection
        .execute_batch(migration)
        .expect("add conference metadata fields");

    let values: (String, String, String, String) = connection
        .query_row(
            "select container_title, conference_name, proceedings_title, place
             from bibliographic_items where id = 'paper-conference-fields'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read migrated item");
    assert_eq!(
        values,
        (
            "Existing container".to_owned(),
            String::new(),
            String::new(),
            String::new()
        )
    );
}

fn insert_run(connection: &Connection, run_id: &str, hash: &str) {
    connection
        .execute(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version,
              prompt_version, output_schema_version, user_goal, scope_hash,
              input_item_count, max_input_tokens, max_output_tokens, timeout_ms,
              cancel_requested, created_at, updated_at)
             values (?1, 'paper_overview', 'needs_review', 'fixture-generation',
                     'fixture-v1', 'pipeline-v1', 'prompt-v1', 1, '', ?2, 1,
                     4096, 1024, 5000, 0, 'now', 'now')",
            (run_id, hash),
        )
        .expect("run");
}

fn seed_v26_ai_graph(connection: &Connection) {
    let hash = "c".repeat(64);
    connection
        .execute_batch(
            "insert into bibliographic_items
             (id, item_type, title, created_at, updated_at)
             values ('paper-migration', 'journalArticle', 'Migration paper', 'now', 'now');
             insert into content_objects
             (sha256, relative_path, mime_type, byte_size, created_at)
             values ('cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'cc/paper.pdf', 'application/pdf', 10, 'now');
             insert into attachments
             (id, item_id, content_sha256, display_name, created_at)
             values ('attachment-migration', 'paper-migration',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'paper.pdf', 'now');
             insert into source_anchors
             (id, item_id, attachment_id, content_sha256, page_index, page_label,
              rects_json, quote_exact, quote_prefix, quote_suffix, created_at)
             values ('anchor-migration', 'paper-migration', 'attachment-migration',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     0, '1', '[]', 'Evidence', '', '', 'now');",
        )
        .expect("source graph");
    insert_run(connection, "run-original", &hash);
    insert_run(connection, "run-retry", &hash);
    connection
        .execute_batch(
            r#"insert into ai_scope_items
             (id, run_id, ordinal, item_kind, object_id, parent_object_id,
              content_sha256, snapshot_hash, created_at)
             values ('scope-migration', 'run-original', 0, 'attachment',
                     'attachment-migration', 'paper-migration',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc', 'now');
             insert into ai_run_stages
             (id, run_id, stage_kind, attempt, status, created_at)
             values ('stage-migration', 'run-original', 'validating', 1, 'succeeded', 'now');
             insert into ai_evidence_bundles
             (id, run_id, scope_hash, retrieval_config_version, validation_status, created_at)
             values ('bundle-migration', 'run-original',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'overview-pages-v1', 'valid', 'now');
             insert into ai_evidence_items
             (id, bundle_id, citation_key, ordinal, anchor_id, paper_id, paper_title,
              attachment_id, content_sha256, page_index, page_label, quote_exact,
              quote_prefix, quote_suffix, context_text, relation_kind,
              validation_status, created_at)
             values ('evidence-migration', 'bundle-migration', 'ev-page-1', 0,
                     'anchor-migration', 'paper-migration', 'Migration paper',
                     'attachment-migration',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     0, '1', 'Evidence', '', '', 'Evidence', 'direct_quote', 'valid', 'now');
             insert into ai_candidates
             (id, run_id, evidence_bundle_id, candidate_kind, schema_version,
              current_revision, review_state, validation_status, original_payload_hash,
              created_at, updated_at)
             values ('candidate-migration', 'run-original', 'bundle-migration',
                     'paper_overview_unit_v1', 1, 1, 'deferred', 'valid',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'now', 'now');
             insert into ai_candidate_revisions
             (id, candidate_id, revision, payload_json, payload_hash, editor_kind,
              source_run_id, parent_revision, change_reason, created_at)
             values ('revision-migration', 'candidate-migration', 1,
                     '{"kind":"paper_overview_unit_v1"}',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'ai', 'run-original', null, 'provider_generation', 'now');
             insert into ai_review_decisions
             (id, candidate_id, candidate_revision, action, rationale, idempotency_key, created_at)
             values ('decision-migration', 'candidate-migration', 1, 'defer', '',
                     'decision-key', 'now');
             insert into ai_evidence_inspections
             (id, candidate_id, candidate_revision, evidence_item_id, viewed_at)
             values ('inspection-migration', 'candidate-migration', 1,
                     'evidence-migration', 'now');
             insert into ai_overview_retry_runs
             (run_id, candidate_id, base_revision, target_fields_json, created_at)
             values ('run-retry', 'candidate-migration', 1, '["limitation"]', 'now');
             insert into ai_overview_profile_runs
             (run_id, paper_id, selection_revision, profile_refs_json,
              field_kinds_json, created_at)
             values ('run-original', 'paper-migration', 1,
                     '[{"profileId":"generic_overview","profileVersion":1}]',
                     '["research_question"]', 'now');
             insert into ai_attention_feedback
             (id, candidate_id, candidate_revision, suggestion_id, feedback_kind,
              note, idempotency_key, created_at)
             values ('feedback-migration', 'candidate-migration', 1, 'q1', 'useful',
                     '', 'feedback-key', 'now');"#,
        )
        .expect("AI graph");
}

#[test]
fn paper_suggestion_runtime_migration_preserves_the_complete_ai_graph() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 26);
    seed_v26_ai_graph(&connection);
    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 27).then_some(*sql))
        .expect("migration 27");
    connection.execute_batch(migration).expect("upgrade to v27");

    for table in [
        "ai_scope_items",
        "ai_run_stages",
        "ai_evidence_bundles",
        "ai_evidence_items",
        "ai_candidates",
        "ai_candidate_revisions",
        "ai_review_decisions",
        "ai_evidence_inspections",
        "ai_overview_retry_runs",
        "ai_overview_profile_runs",
        "ai_attention_feedback",
    ] {
        let sql = format!("select count(*) from {table}");
        let count = connection
            .query_row(&sql, [], |row| row.get::<_, i64>(0))
            .expect("preserved row count");
        assert_eq!(count, 1, "{table}");
    }
    let foreign_key_errors = connection
        .prepare("pragma foreign_key_check")
        .expect("foreign key check")
        .query_map([], |_| Ok(()))
        .expect("foreign key rows")
        .count();
    assert_eq!(foreign_key_errors, 0);
    connection
        .execute(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version,
              prompt_version, output_schema_version, user_goal, scope_hash,
              input_item_count, max_input_tokens, max_output_tokens, timeout_ms,
              cancel_requested, created_at, updated_at)
             values ('run-suggestion', 'paper_suggestions', 'draft', 'fixture-generation',
                     'fixture-v1', 'pipeline-v1', 'prompt-v1', 1, '', ?1, 0,
                     4096, 1024, 5000, 0, 'now', 'now')",
            ["d".repeat(64)],
        )
        .expect("expanded run kind");
    connection
        .execute(
            "insert into ai_suggestion_run_contexts
             (run_id, candidate_kind, created_at)
             values ('run-suggestion', 'concept_suggestion_v1', 'now')",
            [],
        )
        .expect("suggestion context");
}

#[test]
fn research_candidate_migration_preserves_the_complete_ai_graph() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 31);
    seed_v26_ai_graph(&connection);
    connection
        .execute_batch(
            "insert into ai_suggestion_run_contexts
             (run_id, candidate_kind, created_at)
             values ('run-retry', 'concept_suggestion_v1', 'now');
             insert into ai_suggestion_item_decisions
             (id, candidate_id, candidate_revision, suggestion_id, action,
              resolution_kind, rationale, idempotency_key, request_hash, created_at)
             values ('item-decision-migration', 'candidate-migration', 1, 'q1',
                     'defer', 'none', '', 'item-decision-key',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'now');",
        )
        .expect("v31 AI graph");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 32).then_some(*sql))
        .expect("migration 32");
    connection.execute_batch(migration).expect("upgrade to v32");

    for table in [
        "ai_scope_items",
        "ai_run_stages",
        "ai_evidence_bundles",
        "ai_evidence_items",
        "ai_candidates",
        "ai_candidate_revisions",
        "ai_review_decisions",
        "ai_evidence_inspections",
        "ai_overview_retry_runs",
        "ai_overview_profile_runs",
        "ai_attention_feedback",
        "ai_suggestion_run_contexts",
        "ai_suggestion_item_decisions",
    ] {
        let sql = format!("select count(*) from {table}");
        let count = connection
            .query_row(&sql, [], |row| row.get::<_, i64>(0))
            .expect("preserved row count");
        assert_eq!(count, 1, "{table}");
    }
    let foreign_key_errors = connection
        .prepare("pragma foreign_key_check")
        .expect("foreign key check")
        .query_map([], |_| Ok(()))
        .expect("foreign key rows")
        .count();
    assert_eq!(foreign_key_errors, 0);

    connection
        .execute(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version,
              prompt_version, output_schema_version, user_goal, scope_hash,
              input_item_count, max_input_tokens, max_output_tokens, timeout_ms,
              cancel_requested, created_at, updated_at)
             values ('run-reference', 'reference_extraction', 'draft',
                     'fixture-generation', 'fixture-v1', 'pipeline-v1', 'prompt-v1',
                     1, '', ?1, 0, 4096, 1024, 5000, 0, 'now', 'now')",
            ["e".repeat(64)],
        )
        .expect("expanded research task kind");
}

#[test]
fn attachment_reading_state_migration_preserves_state_and_allows_each_attachment() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 35);
    connection
        .execute_batch(
            "insert into bibliographic_items
             (id, item_type, title, created_at, updated_at)
             values ('paper-reading', 'document', 'Reading state', 'now', 'now'),
                    ('paper-reading-stale', 'document', 'Stale merge owner', 'now', 'now');
             insert into content_objects
             (sha256, relative_path, mime_type, byte_size, created_at)
             values ('dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'dd/paper.pdf', 'application/pdf', 10, 'now');
             insert into attachments
             (id, item_id, content_sha256, display_name, created_at)
             values ('attachment-reading-1', 'paper-reading',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'paper-1.pdf', 'now'),
                    ('attachment-reading-2', 'paper-reading',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'paper-2.pdf', 'now');
             insert into reading_states
             (item_id, attachment_id, page_index, zoom, updated_at)
             values ('paper-reading', 'attachment-reading-1', 4, 1.25, 'now'),
                    ('paper-reading-stale', 'attachment-reading-1', 99, 2, 'yesterday');",
        )
        .expect("legacy reading state");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 36).then_some(*sql))
        .expect("migration 36");
    connection.execute_batch(migration).expect("upgrade to v36");
    connection
        .execute(
            "insert into reading_states
             (item_id, attachment_id, page_index, zoom, updated_at)
             values ('paper-reading', 'attachment-reading-2', 8, 1.5, 'now')",
            [],
        )
        .expect("second attachment reading state");

    let states = connection
        .query_row(
            "select count(*), sum(page_index) from reading_states where item_id = 'paper-reading'",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .expect("attachment states");
    assert_eq!(states, (2, 12));
}

#[test]
fn reader_view_state_v2_migration_accepts_v2_and_preserves_legacy_rows() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 36);
    connection
        .execute_batch(
            "insert into bibliographic_items
             (id, item_type, title, created_at, updated_at)
             values ('paper-view-v2', 'document', 'View state', 'now', 'now');
             insert into content_objects
             (sha256, relative_path, mime_type, byte_size, created_at)
             values ('eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'ee/paper.pdf', 'application/pdf', 10, 'now');
             insert into attachments
             (id, item_id, content_sha256, display_name, created_at)
             values ('attachment-view-v2', 'paper-view-v2',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'paper.pdf', 'now');
             insert into reading_states
             (item_id, attachment_id, page_index, zoom, updated_at)
             values ('paper-view-v2', 'attachment-view-v2', 7, 1.25, 'now');",
        )
        .expect("legacy view state");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 38).then_some(*sql))
        .expect("migration 38");
    connection.execute_batch(migration).expect("upgrade to v38");
    let legacy_json: Option<String> = connection
        .query_row(
            "select view_state_json from reading_states where attachment_id = 'attachment-view-v2'",
            [],
            |row| row.get(0),
        )
        .expect("legacy row");
    assert_eq!(legacy_json, None);

    let view_state = r#"{"schemaVersion":2,"pageIndex":7,"scale":{"kind":"page-width"},"top":511,"left":13,"scrollMode":"vertical","spreadMode":"none"}"#;
    connection
        .execute(
            "update reading_states set view_state_json = ?1 where attachment_id = 'attachment-view-v2'",
            [view_state],
        )
        .expect("valid v2 state");
    assert!(
        connection
            .execute(
                "update reading_states set view_state_json = '{\"schemaVersion\":1,\"pageIndex\":7}' where attachment_id = 'attachment-view-v2'",
                [],
            )
            .is_err()
    );
}

fn reader_graph_snapshot(connection: &Connection) -> Vec<String> {
    [
        "select json_group_array(value) from (
             select json_object('attachmentId', attachment_id, 'itemId', item_id,
                 'pageIndex', page_index, 'zoom', zoom, 'updatedAt', updated_at) value
             from reading_states order by attachment_id)",
        "select json_group_array(value) from (
             select json_object('id', id, 'itemId', item_id, 'attachmentId', attachment_id,
                 'sha256', content_sha256, 'selectorVersion', selector_version,
                 'pageIndex', page_index, 'pageLabel', page_label, 'rects', rects_json,
                 'quote', quote_exact, 'prefix', quote_prefix, 'suffix', quote_suffix,
                 'createdAt', created_at) value
             from source_anchors order by id)",
        "select json_group_array(value) from (
             select json_object('id', id, 'type', block_type, 'content', content_json,
                 'props', props_json, 'text', plain_text, 'revision', revision,
                 'status', status, 'createdAt', created_at, 'updatedAt', updated_at) value
             from note_blocks where scope_kind = 'reading' order by id)",
        "select json_group_array(value) from (
             select json_object('blockId', block_id, 'anchorId', current_anchor_id,
                 'paperId', paper_id_snapshot, 'attachmentId', attachment_id_snapshot,
                 'sha256', content_sha256, 'pageIndex', page_index, 'pageLabel', page_label,
                 'rects', rects_json, 'quote', quote_exact, 'prefix', quote_prefix,
                 'suffix', quote_suffix, 'locatorVersion', locator_version,
                 'createdAt', created_at, 'updatedAt', updated_at) value
             from reading_note_sources order by block_id)",
    ]
    .iter()
    .map(|sql| {
        connection
            .query_row(sql, [], |row| row.get(0))
            .expect("reader graph snapshot")
    })
    .collect()
}

#[test]
fn reader_v2_migrations_preserve_v1_graph_and_round_trip_v2_payloads() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 37);
    let sha256 = "f".repeat(64);
    connection
        .execute_batch(&format!(
            r##"insert into bibliographic_items
               (id, item_type, title, created_at, updated_at)
               values ('paper-reader-v1', 'document', 'Reader migration', 'before', 'before');
               insert into content_objects
               (sha256, relative_path, mime_type, byte_size, created_at)
               values ('{sha256}', 'ff/reader.pdf', 'application/pdf', 10, 'before');
               insert into attachments
               (id, item_id, content_sha256, display_name, created_at)
               values ('attachment-reader-v1', 'paper-reader-v1', '{sha256}', 'reader.pdf', 'before');
               insert into reading_states
               (attachment_id, item_id, page_index, zoom, updated_at)
               values ('attachment-reader-v1', 'paper-reader-v1', 8, 1.375, 'before');
               insert into source_anchors
               (id, item_id, attachment_id, content_sha256, selector_version, page_index,
                page_label, rects_json, quote_exact, quote_prefix, quote_suffix, created_at)
               values ('anchor-reader-v1', 'paper-reader-v1', 'attachment-reader-v1',
                       '{sha256}', 1, 8, 'ix', '[{{"x":1,"y":2,"width":3,"height":4}}]',
                       'stable quote', 'before ', ' after', 'before');
               insert into note_blocks
               (id, scope_kind, position, block_type, props_json, content_json, plain_text,
                schema_version, revision, status, is_favorite, created_at, updated_at)
               values ('note-reader-v1', 'reading', 0, 'reading_annotation',
                       '{{"color":"#f4cf65"}}', '{{"body":"draft body"}}', 'draft body',
                       1, 3, 'active', 0, 'before', 'before');
               insert into reading_note_sources
               (block_id, current_anchor_id, paper_id_snapshot, paper_title_snapshot,
                attachment_id_snapshot, attachment_name_snapshot, content_sha256, page_index,
                page_label, rects_json, quote_exact, quote_prefix, quote_suffix,
                locator_version, created_at, updated_at)
               values ('note-reader-v1', 'anchor-reader-v1', 'paper-reader-v1',
                       'Reader migration', 'attachment-reader-v1', 'reader.pdf', '{sha256}',
                       8, 'ix', '[{{"x":1,"y":2,"width":3,"height":4}}]',
                       'stable quote', 'before ', ' after', 1, 'before', 'before');"##,
        ))
        .expect("legacy reader graph");
    let before = reader_graph_snapshot(&connection);

    for version in [38, 39] {
        let migration = MIGRATIONS
            .iter()
            .find_map(|(candidate, sql)| (*candidate == version).then_some(*sql))
            .expect("reader migration");
        let transaction = connection.transaction().expect("migration transaction");
        transaction
            .execute_batch(migration)
            .expect("upgrade reader schema");
        transaction
            .execute(
                "insert into schema_migrations (version, applied_at) values (?1, 'after')",
                [version],
            )
            .expect("record reader migration");
        transaction.commit().expect("commit reader migration");
    }

    assert_eq!(reader_graph_snapshot(&connection), before);
    let counts: (i64, i64, i64, i64) = connection
        .query_row(
            "select (select count(*) from reading_states),
                    (select count(*) from source_anchors),
                    (select count(*) from note_blocks where scope_kind = 'reading'),
                    (select count(*) from reading_note_sources)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("reader graph counts");
    assert_eq!(counts, (1, 1, 1, 1));
    let legacy_versions: (i64, i64, Option<String>, Option<String>, Option<String>) = connection
        .query_row(
            "select a.selector_version, s.locator_version, r.view_state_json,
                    a.selector_json, s.selector_json
             from source_anchors a
             join reading_note_sources s on s.current_anchor_id = a.id
             join reading_states r on r.attachment_id = a.attachment_id",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("legacy compatibility columns");
    assert_eq!(legacy_versions, (1, 1, None, None, None));

    let view_state = r#"{"schemaVersion":2,"pageIndex":8,"scale":{"kind":"numeric","value":1.375},"top":240,"left":12,"scrollMode":"vertical","spreadMode":"none"}"#;
    let selector = format!(
        r#"{{"schemaVersion":2,"attachmentSha256":"{sha256}","anchorId":"anchor-reader-v1","segments":[{{"pageIndex":8,"pageLabel":"ix","rects":[{{"x":1,"y":2,"width":3,"height":4}}],"exactText":"stable quote","prefix":"before ","suffix":" after"}}]}}"#,
    );
    connection
        .execute(
            "update reading_states set view_state_json = ?1 where attachment_id = 'attachment-reader-v1'",
            [view_state],
        )
        .expect("write V2 view state");
    connection
        .execute(
            "update source_anchors set selector_version = 2, selector_json = ?1 where id = 'anchor-reader-v1'",
            [&selector],
        )
        .expect("write V2 source anchor");
    connection
        .execute(
            "update reading_note_sources set locator_version = 2, selector_json = ?1 where block_id = 'note-reader-v1'",
            [&selector],
        )
        .expect("write V2 reading-note source");
    let round_trip: (String, String, String) = connection
        .query_row(
            "select r.view_state_json, a.selector_json, s.selector_json
             from reading_states r
             join source_anchors a on a.attachment_id = r.attachment_id
             join reading_note_sources s on s.current_anchor_id = a.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("V2 round trip");
    assert_eq!(
        round_trip,
        (view_state.to_owned(), selector.clone(), selector)
    );
    let foreign_key_errors = connection
        .prepare("pragma foreign_key_check")
        .expect("foreign key check")
        .query_map([], |_| Ok(()))
        .expect("foreign key rows")
        .count();
    assert_eq!(foreign_key_errors, 0);
}

#[test]
fn paper_ai_model_usage_migration_preserves_turns_and_accepts_model_snapshots() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 50);
    connection
        .execute_batch(
            "insert into bibliographic_items
                 (id, item_type, title, created_at, updated_at)
             values ('paper-ai-model-usage', 'journalArticle', 'AI model usage', 'before', 'before');
             insert or ignore into llm_provider_configs
                 (provider_id, enabled, base_url, timeout_ms, updated_at)
             values ('qwen', 1, '', 60000, 'before');
             insert into paper_ai_reading_sessions
                 (id, paper_id, attachment_ids_json, parse_run_ids_json, provider_id,
                  model_id, title, status, created_at, updated_at)
             values ('session-ai-model-usage', 'paper-ai-model-usage', '[]', '[]', 'qwen',
                     'qwen-vl', 'Existing session', 'active', 'before', 'before');
             insert into paper_ai_reading_turns
                 (id, session_id, ordinal, user_message, assistant_message, provider_id,
                  model_id, status, input_tokens, output_tokens, created_at, finished_at)
             values ('turn-ai-model-usage', 'session-ai-model-usage', 0, 'Question', 'Answer',
                     'qwen', 'qwen-vl', 'succeeded', 128, 32, 'before', 'before');",
        )
        .expect("seed paper AI reading records");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 51).then_some(*sql))
        .expect("migration 51");
    connection
        .execute_batch(migration)
        .expect("upgrade paper AI model usage schema");

    let preserved: (String, String, Option<String>, i64, Option<i64>) = connection
        .query_row(
            "select s.model_id, t.assistant_message, t.reasoning_effort,
                    t.input_tokens, t.context_window_tokens
             from paper_ai_reading_sessions s
             join paper_ai_reading_turns t on t.session_id = s.id
             where s.id = 'session-ai-model-usage'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("legacy paper AI reading data");
    assert_eq!(
        preserved,
        ("qwen-vl".to_owned(), "Answer".to_owned(), None, 128, None)
    );

    connection
        .execute(
            "update paper_ai_reading_sessions
             set reasoning_effort = 'high'
             where id = 'session-ai-model-usage'",
            [],
        )
        .expect("write latest session model selection");
    connection
        .execute(
            "update paper_ai_reading_turns
             set reasoning_effort = 'high', context_window_tokens = 32768,
                 context_window_source = 'models_dev'
             where id = 'turn-ai-model-usage'",
            [],
        )
        .expect("write turn model snapshot");

    let snapshot: (Option<String>, Option<String>, Option<i64>, Option<String>) = connection
        .query_row(
            "select s.reasoning_effort, t.reasoning_effort, t.context_window_tokens,
                    t.context_window_source
             from paper_ai_reading_sessions s
             join paper_ai_reading_turns t on t.session_id = s.id
             where t.id = 'turn-ai-model-usage'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("model snapshot");
    assert_eq!(
        snapshot,
        (
            Some("high".to_owned()),
            Some("high".to_owned()),
            Some(32768),
            Some("models_dev".to_owned())
        )
    );
}

#[test]
fn paper_ai_turn_cancellation_migration_preserves_snapshots_and_accepts_canceled_status() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 51);
    connection
        .execute_batch(
            r#"insert into bibliographic_items
                 (id, item_type, title, created_at, updated_at)
             values ('paper-ai-cancel', 'journalArticle', 'Cancelable turn', 'before', 'before');
             insert or ignore into llm_provider_configs
                 (provider_id, enabled, base_url, timeout_ms, updated_at)
             values ('qwen', 1, '', 60000, 'before');
             insert into paper_ai_reading_sessions
                 (id, paper_id, attachment_ids_json, parse_run_ids_json, provider_id,
                  model_id, reasoning_effort, title, status, created_at, updated_at)
             values ('session-ai-cancel', 'paper-ai-cancel', '[]', '[]', 'qwen',
                     'qwen-vl', 'high', 'Cancelable session', 'active', 'before', 'before');
             insert into paper_ai_reading_turns
                 (id, session_id, ordinal, user_message, assistant_message, provider_id,
                  model_id, status, input_tokens, output_tokens, raw_request_json,
                  raw_response_json, reasoning_effort, context_window_tokens,
                  context_window_source, created_at, finished_at)
             values ('turn-ai-cancel', 'session-ai-cancel', 0, 'Question', 'Partial answer',
                     'qwen', 'qwen-vl', 'running', 128, 12, '{"messages":[]}',
                     '{"events":[]}', 'high', 32768, 'models_dev', 'before', null);"#,
        )
        .expect("seed cancelable paper AI turn");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 52).then_some(*sql))
        .expect("migration 52");
    connection
        .execute_batch("pragma foreign_keys = off;")
        .expect("disable foreign keys");
    let transaction = connection.transaction().expect("migration transaction");
    transaction
        .execute_batch(migration)
        .expect("allow canceled paper AI turns");
    let foreign_key_errors = transaction
        .prepare("pragma foreign_key_check")
        .expect("foreign key check")
        .query_map([], |_| Ok(()))
        .expect("foreign key rows")
        .count();
    assert_eq!(foreign_key_errors, 0);
    transaction.commit().expect("commit migration");
    connection
        .execute_batch("pragma foreign_keys = on;")
        .expect("restore foreign keys");
    let context_turn_parent: String = connection
        .query_row(
            "select \"table\" from pragma_foreign_key_list('paper_ai_reading_turn_context_items')
             where \"from\" = 'turn_id'",
            [],
            |row| row.get(0),
        )
        .expect("context turn foreign key");
    assert_eq!(context_turn_parent, "paper_ai_reading_turns");

    connection
        .execute(
            "update paper_ai_reading_turns
             set status = 'canceled', error_code = 'canceled', finished_at = 'after'
             where id = 'turn-ai-cancel'",
            [],
        )
        .expect("persist canceled status");
    let preserved: (String, String, String, String, Option<i64>) = connection
        .query_row(
            "select status, assistant_message, raw_request_json, reasoning_effort,
                    context_window_tokens
             from paper_ai_reading_turns where id = 'turn-ai-cancel'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("read canceled turn");
    assert_eq!(
        preserved,
        (
            "canceled".to_owned(),
            "Partial answer".to_owned(),
            r#"{"messages":[]}"#.to_owned(),
            "high".to_owned(),
            Some(32768)
        )
    );
    assert!(
        connection
            .execute(
                "update paper_ai_reading_turns set status = 'interrupted' where id = 'turn-ai-cancel'",
                [],
            )
            .is_err()
    );
}

#[test]
fn paper_ai_turn_messages_migration_preserves_legacy_conversation_order() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 52);
    connection
        .execute_batch(
            r#"insert into bibliographic_items
                 (id, item_type, title, created_at, updated_at)
             values ('paper-ai-messages', 'journalArticle', 'Message history', 'before', 'before');
             insert or ignore into llm_provider_configs
                 (provider_id, enabled, base_url, timeout_ms, updated_at)
             values ('qwen', 1, '', 60000, 'before');
             insert into paper_ai_reading_sessions
                 (id, paper_id, attachment_ids_json, parse_run_ids_json, provider_id,
                  model_id, title, status, created_at, updated_at)
             values ('session-ai-messages', 'paper-ai-messages', '[]', '[]', 'qwen',
                     'qwen-vl', 'History', 'active', 'before', 'before');
             insert into paper_ai_reading_turns
                 (id, session_id, ordinal, user_message, assistant_message, provider_id,
                  model_id, status, created_at)
             values ('turn-with-answer', 'session-ai-messages', 0, 'Question', 'Answer',
                     'qwen', 'qwen-vl', 'succeeded', 'before');
             insert into paper_ai_reading_turns
                 (id, session_id, ordinal, user_message, assistant_message, provider_id,
                  model_id, status, created_at)
             values ('turn-without-answer', 'session-ai-messages', 1, 'Pending', '',
                     'qwen', 'qwen-vl', 'failed', 'before');"#,
        )
        .expect("seed legacy paper AI turns");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 53).then_some(*sql))
        .expect("migration 53");
    connection
        .execute_batch(migration)
        .expect("add ordered paper AI turn messages");

    let answered: String = connection
        .query_row(
            "select messages_json from paper_ai_reading_turns where id = 'turn-with-answer'",
            [],
            |row| row.get(0),
        )
        .expect("answered messages");
    let pending: String = connection
        .query_row(
            "select messages_json from paper_ai_reading_turns where id = 'turn-without-answer'",
            [],
            |row| row.get(0),
        )
        .expect("pending messages");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&answered).expect("answered json"),
        serde_json::json!([
            { "id": "turn-with-answer-user-0", "role": "user", "content": "Question" },
            { "id": "turn-with-answer-assistant-0", "role": "assistant", "content": "Answer" }
        ])
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&pending).expect("pending json"),
        serde_json::json!([
            { "id": "turn-without-answer-user-0", "role": "user", "content": "Pending" }
        ])
    );
}

#[test]
fn docling_daemon_settings_migration_requires_conda_and_removes_legacy_command() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 53);
    connection
        .execute(
            "update docling_settings
             set executable = 'custom-docling', conda_environment = '',
                 idle_shutdown_ms = 600000, request_timeout_ms = 120000
             where id = 1",
            [],
        )
        .expect("seed legacy Docling settings");

    let migration = MIGRATIONS
        .iter()
        .find_map(|(version, sql)| (*version == 54).then_some(*sql))
        .expect("migration 54");
    connection
        .execute_batch(migration)
        .expect("migrate Docling daemon settings");

    let settings: (String, u64, u64) = connection
        .query_row(
            "select conda_environment, idle_shutdown_ms, request_timeout_ms
             from docling_settings where id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read migrated settings");
    assert_eq!(settings, ("research".to_owned(), 600_000, 120_000));

    let has_executable: bool = connection
        .query_row(
            "select exists(
                 select 1 from pragma_table_info('docling_settings') where name = 'executable'
             )",
            [],
            |row| row.get(0),
        )
        .expect("inspect migrated columns");
    assert!(!has_executable);
}

#[test]
fn semantic_vector_control_plane_enforces_spaces_generations_and_ready_state() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 55);
    connection
        .execute(
            "insert into embedding_spaces
             (id, provider_id, region, model_id, model_revision, dimension, scalar_type,
              distance_metric, normalization, document_input_role, query_input_role,
              instruction_set_version, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, 'float32', 'cosine', 'l2',
                     'document', 'query', 'concept-v1', 'now')",
            (
                "space-1",
                "qwen",
                "cn-beijing",
                "text-embedding-v4",
                "2026-08",
                1024,
            ),
        )
        .expect("embedding space");
    connection
        .execute(
            "insert into semantic_representations
             (id, subject_kind, subject_id, subject_revision, representation_kind,
              namespace, language, render_version, rendered_text, input_sha256, created_at)
             values (?1, 'concept', ?2, 1, 'concept_identity', 'accepted_concepts',
                     'en', 'concept-identity-v1', ?3, ?4, 'now')",
            (
                "representation-1",
                "concept-1",
                "preferred label: X\nsemantic identity: a test identity",
                "a".repeat(64),
            ),
        )
        .expect("semantic representation");
    connection
        .execute(
            "insert into embedding_index_generations
             (id, namespace, embedding_space_id, backend_kind, generation, status,
              relative_database_path, lance_table, index_kind, config_json, created_at)
             values (?1, 'accepted_concepts', 'space-1', 'lancedb', 1, 'active',
                     'vector-indexes/space-1/generation-000001', 'accepted_concepts',
                     'exact', '{}', 'now')",
            ["generation-invalid"],
        )
        .expect_err("active generation needs activation timestamp");
    connection
        .execute(
            "insert into embedding_index_generations
             (id, namespace, embedding_space_id, backend_kind, generation, status,
              relative_database_path, lance_table, lance_dataset_version, index_kind,
              config_json, created_at, activated_at)
             values (?1, 'accepted_concepts', 'space-1', 'lancedb', 1, 'active',
                     'vector-indexes/space-1/generation-000001', 'accepted_concepts', 1,
                     'exact', '{}', 'now', 'now')",
            ["generation-1"],
        )
        .expect("active generation");
    connection
        .execute(
            "insert into embedding_index_generations
             (id, namespace, embedding_space_id, backend_kind, generation, status,
              relative_database_path, lance_table, index_kind, config_json, created_at, activated_at)
             values (?1, 'accepted_concepts', 'space-1', 'lancedb', 2, 'active',
                     'vector-indexes/space-1/generation-000002', 'accepted_concepts',
                     'exact', '{}', 'now', 'now')",
            ["generation-2"],
        )
        .expect_err("one active generation per namespace and space");
    connection
        .execute(
            "insert into embedding_records
             (id, representation_id, embedding_space_id, index_generation_id,
              lance_table, lance_row_id, vector_encoding, status, created_at, updated_at)
             values (?1, 'representation-1', 'space-1', 'generation-1',
                     'accepted_concepts', 'row-1', 'float32', 'ready', 'now', 'now')",
            ["record-invalid"],
        )
        .expect_err("ready record needs vector verification metadata");
    connection
        .execute(
            "insert into embedding_records
             (id, representation_id, embedding_space_id, index_generation_id,
              lance_table, lance_row_id, lance_dataset_version, vector_encoding,
              vector_norm, status, created_at, updated_at)
             values (?1, 'representation-1', 'space-1', 'generation-1',
                     'accepted_concepts', 'row-1', 1, 'float32', 1.0,
                     'ready', 'now', 'now')",
            ["record-1"],
        )
        .expect("verified ready record");
    connection
        .execute(
            "update semantic_representations set rendered_text = 'changed' where id = ?1",
            ["representation-1"],
        )
        .expect_err("semantic representation is immutable");
    connection
        .execute(
            "insert into embedding_index_generations
             (id, namespace, embedding_space_id, backend_kind, generation, status,
              relative_database_path, lance_table, index_kind, config_json, created_at)
             values (?1, 'source_passages', 'space-1', 'lancedb', 1, 'building',
                     '../escape', 'source_passages', 'exact', '{}', 'now')",
            ["generation-path-invalid"],
        )
        .expect_err("generation path traversal is rejected");
}

#[test]
fn semantic_generation_identity_allows_parallel_rebuilds_and_same_text_concepts() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 55);
    connection
        .execute_batch(
            "insert into embedding_spaces
             (id, provider_id, region, model_id, model_revision, dimension, scalar_type,
              distance_metric, normalization, document_input_role, query_input_role,
              instruction_set_version, created_at)
             values ('space-migrate', 'qwen-embedding', 'cn-beijing', 'text-embedding-v4',
                     'v4', 3, 'float32', 'cosine', 'l2', 'document', 'query', 'v1', 'now');
             insert into semantic_representations
             (id, subject_kind, subject_id, subject_revision, representation_kind,
              namespace, language, render_version, rendered_text, input_sha256, created_at)
             values ('representation-a', 'concept', 'concept-a', 1, 'concept_identity',
                     'accepted_concepts', 'zh', 'v1', 'shared wording',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'now');
             insert into embedding_index_generations
             (id, namespace, embedding_space_id, backend_kind, generation, status,
              relative_database_path, lance_table, index_kind, config_json, created_at)
             values ('generation-a', 'accepted_concepts', 'space-migrate', 'lancedb', 1,
                     'building', 'space-migrate/generation-000001', 'accepted_concepts',
                     'exact', '{}', 'now');
             insert into embedding_records
             (id, representation_id, embedding_space_id, index_generation_id, lance_table,
              lance_row_id, vector_encoding, status, created_at, updated_at)
             values ('record-a', 'representation-a', 'space-migrate', 'generation-a',
                     'accepted_concepts', 'row-a', 'float32', 'queued', 'now', 'now');
             insert into embedding_outbox_jobs
             (id, embedding_record_id, attempt, status, idempotency_key, created_at)
             values ('job-a', 'record-a', 1, 'queued', 'job-a-key', 'now');",
        )
        .expect("v55 fixture");

    connection
        .execute_batch("pragma foreign_keys = off;")
        .expect("disable foreign keys for table rebuild");
    connection
        .execute_batch(include_str!(
            "../../migrations/0056_semantic_generation_identity.sql"
        ))
        .expect("apply semantic generation identity migration");
    connection
        .execute_batch("pragma foreign_keys = on;")
        .expect("restore foreign keys");
    connection
        .execute(
            "insert into semantic_representations
             (id, subject_kind, subject_id, subject_revision, representation_kind,
              namespace, language, render_version, rendered_text, input_sha256, created_at)
             values ('representation-b', 'concept', 'concept-b', 1, 'concept_identity',
                     'accepted_concepts', 'zh', 'v1', 'shared wording', ?1, 'now')",
            ["a".repeat(64)],
        )
        .expect("different concepts may render identical text");
    connection
        .execute_batch(
            "insert into embedding_index_generations
             (id, namespace, embedding_space_id, backend_kind, generation, status,
              relative_database_path, lance_table, index_kind, config_json, created_at)
             values ('generation-b', 'accepted_concepts', 'space-migrate', 'lancedb', 2,
                     'building', 'space-migrate/generation-000002', 'accepted_concepts',
                     'exact', '{}', 'now');
             insert into embedding_records
             (id, representation_id, embedding_space_id, index_generation_id, lance_table,
              lance_row_id, vector_encoding, status, created_at, updated_at)
             values ('record-b', 'representation-a', 'space-migrate', 'generation-b',
                     'accepted_concepts', 'row-b', 'float32', 'queued', 'now', 'now');",
        )
        .expect("same representation can join a rebuilding generation");
    assert_eq!(
        connection
            .query_row("select count(*) from embedding_outbox_jobs", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn paper_analysis_migration_preserves_ai_candidates_and_enables_concept_runs() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 57);
    connection
        .execute_batch(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
              output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
              max_output_tokens, timeout_ms, created_at, updated_at)
             values ('run-before-analysis', 'paper_overview', 'needs_review',
                     'fixture-generation', 'fixture-v1', 'pipeline-v1', 'prompt-v1', 1,
                     '', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     0, 1000, 1000, 5000, 'now', 'now');
             insert into ai_evidence_bundles
             (id, run_id, scope_hash, retrieval_config_version, validation_status, created_at)
             values ('bundle-before-analysis', 'run-before-analysis',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'fixture-v1', 'valid', 'now');
             insert into ai_candidates
             (id, run_id, evidence_bundle_id, candidate_kind, schema_version,
              current_revision, review_state, validation_status, original_payload_hash,
              created_at, updated_at)
             values ('candidate-before-analysis', 'run-before-analysis',
                     'bundle-before-analysis', 'paper_overview_unit_v1', 1, 1,
                     'unreviewed', 'valid',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     'now', 'now');",
        )
        .expect("v57 AI fixture");

    connection
        .execute_batch("pragma foreign_keys = off;")
        .expect("disable foreign keys for table rebuild");
    connection
        .execute_batch(include_str!(
            "../../migrations/0058_paper_analysis_workflows.sql"
        ))
        .expect("apply paper analysis workflow migration");
    connection
        .execute_batch("pragma foreign_keys = on;")
        .expect("restore foreign keys");

    assert_eq!(
        connection
            .query_row(
                "select candidate_kind from ai_candidates where id = 'candidate-before-analysis'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "paper_overview_unit_v1"
    );
    connection
        .execute(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
              output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
              max_output_tokens, timeout_ms, created_at, updated_at)
             values ('concept-run', 'paper_concept_map', 'queued', 'fixture-generation',
                     'fixture-v1', 'pipeline-v1', 'prompt-v1', 1, '', ?1, 0,
                     1000, 1000, 5000, 'now', 'now')",
            ["c".repeat(64)],
        )
        .expect("concept task kind is accepted");
    assert_eq!(
        connection
            .prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn paper_concept_acceptance_migration_preserves_relations_and_adds_graph_predicates() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 58);
    connection
        .execute_batch(
            "insert into relation_assertions
             (id, subject_kind, subject_id, subject_label_snapshot, predicate_kind,
              object_kind, object_id, object_label_snapshot, scope_note, provenance_kind,
              epistemic_role, revision, created_at, updated_at)
             values ('relation-before-concepts', 'concept', 'concept-a', 'A', 'related_to',
                     'concept', 'concept-b', 'B', '', 'human_original', 'assertion', 1,
                     'now', 'now');",
        )
        .expect("v58 relation fixture");

    connection
        .execute_batch("pragma foreign_keys = off;")
        .expect("disable foreign keys for relation rebuild");
    connection
        .execute_batch(include_str!(
            "../../migrations/0059_paper_concept_acceptance.sql"
        ))
        .expect("apply paper concept acceptance migration");
    connection
        .execute_batch("pragma foreign_keys = on;")
        .expect("restore foreign keys");

    assert_eq!(
        connection
            .query_row(
                "select predicate_kind from relation_assertions
                 where id = 'relation-before-concepts'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "related_to"
    );
    connection
        .execute(
            "insert into relation_assertions
             (id, subject_kind, subject_id, subject_label_snapshot, predicate_kind,
              object_kind, object_id, object_label_snapshot, scope_note, provenance_kind,
              epistemic_role, revision, created_at, updated_at)
             values ('relation-concept-graph', 'concept', 'concept-a', 'A', 'depends_on',
                     'concept', 'concept-b', 'B', '', 'human_original', 'assertion', 1,
                     'now', 'now')",
            [],
        )
        .expect("concept graph predicate");
    assert_eq!(
        connection
            .prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn paper_reading_bootstrap_migration_marks_legacy_sessions_fresh_and_immutable() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 59);
    connection
        .execute_batch(
            "insert into bibliographic_items (id, item_type, title, created_at, updated_at)
             values ('bootstrap-paper', 'journalArticle', 'Paper', 'now', 'now');",
        )
        .unwrap();
    let provider_id: String = connection
        .query_row(
            "select provider_id from llm_provider_configs order by provider_id limit 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            "insert into paper_ai_reading_sessions
             (id, paper_id, attachment_ids_json, parse_run_ids_json, provider_id, model_id,
              title, status, created_at, updated_at)
             values ('legacy-bootstrap-session', 'bootstrap-paper', '[]', '[]', ?1, 'model',
                     'Legacy', 'active', 'now', 'now')",
            [provider_id],
        )
        .unwrap();

    connection
        .execute_batch(include_str!(
            "../../migrations/0060_paper_reading_bootstrap.sql"
        ))
        .expect("apply bootstrap migration");
    let migrated: (String, String, Option<String>) = connection
        .query_row(
            "select bootstrap_kind, bootstrap_messages_json, bootstrap_hash
             from paper_ai_reading_sessions where id = 'legacy-bootstrap-session'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(migrated, ("fresh_paper".to_owned(), "[]".to_owned(), None));
    assert!(
        connection
            .execute(
                "update paper_ai_reading_sessions set bootstrap_kind = 'structured_history'
                 where id = 'legacy-bootstrap-session'",
                [],
            )
            .is_err(),
        "bootstrap identity is immutable after session creation",
    );
}

#[test]
fn concept_retrieval_migration_indexes_current_concepts_and_constrains_traces() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 60);
    connection
        .execute_batch(
            "insert into terms
             (id, text, normalized_text, language, provenance_kind, revision, created_at, updated_at)
             values ('retrieval-term', 'Semantic Network', 'semantic network', 'en',
                     'human_original', 1, 'now', 'now');
             insert into concepts
             (id, label, explanation, disambiguation, domain, tradition, period,
              provenance_kind, revision, created_at, updated_at)
             values ('retrieval-concept', 'Semantic Network', 'A graph of accepted concepts',
                     '', 'knowledge representation', '', '', 'human_original', 1, 'now', 'now');
             insert into concept_terms(concept_id, term_id, relation_kind, created_at)
             values ('retrieval-concept', 'retrieval-term', 'preferred', 'now');",
        )
        .expect("seed legacy concept");

    connection
        .execute_batch(include_str!(
            "../../migrations/0061_concept_retrieval_traces.sql"
        ))
        .expect("apply concept retrieval migration");
    let concept_id: String = connection
        .query_row(
            "select concept_id from concept_search where concept_search match 'semantic'",
            [],
            |row| row.get(0),
        )
        .expect("backfilled FTS concept");
    assert_eq!(concept_id, "retrieval-concept");

    connection
        .execute(
            "update terms set text = 'Knowledge Graph', updated_at = 'later'
             where id = 'retrieval-term'",
            [],
        )
        .expect("update preferred term");
    let updated: String = connection
        .query_row(
            "select concept_id from concept_search where concept_search match 'knowledge'",
            [],
            |row| row.get(0),
        )
        .expect("trigger refreshed FTS concept");
    assert_eq!(updated, "retrieval-concept");

    connection
        .execute(
            "insert into concept_retrieval_traces
             (id, query_sha256, status, config_json, context_text, context_sha256,
              seed_count, edge_count, source_count, external_paper_count, created_at, expires_at)
             values ('trace-preview', ?1, 'previewed', '{}', '', ?1, 0, 0, 0, 0, 'now', 'later')",
            ["a".repeat(64)],
        )
        .expect("valid retrieval trace");
    assert!(
        connection
            .execute(
                "update concept_retrieval_traces set status = 'sent'
                 where id = 'trace-preview'",
                [],
            )
            .is_err(),
        "sent traces require a sent timestamp",
    );
}

#[test]
fn concept_algorithm_migration_keeps_derived_runs_separate_from_formal_relations() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 61);
    connection
        .execute_batch(include_str!(
            "../../migrations/0062_concept_algorithm_runs.sql"
        ))
        .expect("apply concept algorithm migration");
    connection
        .execute(
            "insert into concept_algorithm_runs
             (id, algorithm_kind, algorithm_id, algorithm_version, parameters_json,
              parameters_hash, concept_revision_watermark, relation_revision_watermark,
              paper_scope_json, paper_scope_hash, status, result_count, created_at,
              started_at, finished_at)
             values ('algorithm-run', 'shared_foundations', 'weighted-jaccard', 1, '{}',
                     ?1, 0, 0, '[]', ?1, 'succeeded', 0, 'now', 'now', 'now')",
            ["a".repeat(64)],
        )
        .expect("valid derived run");
    assert!(
        connection
            .execute(
                "insert into concept_algorithm_runs
                 (id, algorithm_kind, algorithm_id, algorithm_version, parameters_json,
                  parameters_hash, concept_revision_watermark, relation_revision_watermark,
                  paper_scope_json, paper_scope_hash, status, created_at, started_at)
                 values ('community-run', 'community_detection', 'community', 1, '{}',
                         ?1, 0, 0, '[]', ?1, 'running', 'now', 'now')",
                ["b".repeat(64)],
            )
            .is_err(),
        "community detection is not an accepted algorithm kind",
    );
    let relation_count: i64 = connection
        .query_row("select count(*) from relation_assertions", [], |row| {
            row.get(0)
        })
        .expect("formal relation count");
    assert_eq!(relation_count, 0);
}

#[test]
fn agent_legacy_migrations_are_registered_as_one_upgrade_chain() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_through(&mut connection, 108);
    let versions: Vec<i64> = connection
        .prepare("select version from schema_migrations where version >= 105 order by version")
        .expect("schema migration rows")
        .query_map([], |row| row.get(0))
        .expect("schema migration query")
        .collect::<Result<_, _>>()
        .expect("schema migration versions");
    assert_eq!(versions, vec![105, 106, 107, 108]);

    let columns: Vec<String> = connection
        .prepare("pragma table_info(agent_legacy_ai_run_bindings)")
        .expect("legacy binding table")
        .query_map([], |row| row.get(1))
        .expect("legacy binding columns")
        .collect::<Result<_, _>>()
        .expect("legacy binding column names");
    assert!(columns.iter().any(|column| column == "terminal_status"));
    assert!(columns.iter().any(|column| column == "candidate_id"));
    let legacy_session_table: i64 = connection
        .query_row(
            "select count(*) from sqlite_master where type='table' and name='agent_paper_reading_legacy_sessions'",
            [],
            |row| row.get(0),
        )
        .expect("legacy reading metadata table");
    let legacy_turn_table: i64 = connection
        .query_row(
            "select count(*) from sqlite_master where type='table' and name='agent_paper_reading_legacy_turns'",
            [],
            |row| row.get(0),
        )
        .expect("legacy reading turn metadata table");
    assert_eq!((legacy_session_table, legacy_turn_table), (1, 1));
}

#[path = "storage_migration_tests/agent_change_sets.rs"]
mod agent_change_sets;
#[path = "storage_migration_tests/agent_object_staging.rs"]
mod agent_object_staging;
#[path = "storage_migration_tests/agent_proposal_applications.rs"]
mod agent_proposal_applications;
#[path = "storage_migration_tests/agent_workspace_discards.rs"]
mod agent_workspace_discards;
#[path = "storage_migration_tests/apply_restoration_upgrade.rs"]
mod apply_restoration_upgrade;
#[path = "storage_migration_tests/context_fragments_upgrade.rs"]
mod context_fragments_upgrade;
#[path = "storage_migration_tests/file_operation_ownership.rs"]
mod file_operation_ownership;
#[path = "storage_migration_tests/undo_restoration_upgrade.rs"]
mod undo_restoration_upgrade;
