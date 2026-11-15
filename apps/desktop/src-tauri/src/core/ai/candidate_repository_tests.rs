use rusqlite::Connection;

use super::*;
use crate::storage::migrate;

use super::super::candidate_test_fixture::{
    insert_selection_source, save as save_generated_candidate, seed, selection_payload,
};

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open database");
    migrate(&mut connection).expect("apply migrations");
    connection
}

#[test]
fn paper_scope_filters_before_loading_other_papers_history() {
    let mut connection = database();
    let (anchor_id, _) = insert_selection_source(&connection);
    let (run, bundle) = seed(&connection, &anchor_id);
    let own = save_generated_candidate(
        &mut connection,
        &run.id,
        &bundle,
        &selection_payload("当前论文的候选。").to_string(),
    )
    .expect("own candidate");
    let (other_run, other_bundle) = seed(&connection, &anchor_id);
    let other = save_generated_candidate(
        &mut connection,
        &other_run.id,
        &other_bundle,
        &selection_payload("其他论文的候选。").to_string(),
    )
    .expect("other candidate");
    connection
        .execute_batch(
            "insert into bibliographic_items
        (id,item_type,title,created_at,updated_at)
        values ('other-paper','journalArticle','Other Paper','now','now');",
        )
        .expect("other paper");
    connection
        .execute(
            "update ai_evidence_items set paper_id='other-paper' where bundle_id=?1",
            [&other_bundle.id],
        )
        .expect("other paper sources");
    // A foreign history that cannot deserialize must not affect a scoped read.
    connection
        .execute_batch("pragma ignore_check_constraints=on;")
        .expect("fixture injection");
    connection
        .execute(
            "insert into ai_candidate_revisions
             (id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,parent_revision,change_reason,created_at)
             select 'unreadable-foreign-history',candidate_id,2,'invalid json',payload_hash,'human',null,1,'fixture','now'
             from ai_candidate_revisions where candidate_id=?1 and revision=1",
            [&other.id],
        )
        .expect("unreadable foreign history");
    connection
        .execute_batch("pragma ignore_check_constraints=off;")
        .expect("restore checks");
    assert!(list_candidates(&connection, None, None).is_err());
    let scoped = list_candidates(&connection, None, Some("paper-ai")).expect("scoped read");
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].id, own.id);
    assert!(!scoped[0].revisions.is_empty());
    assert_eq!(
        list_candidates(&connection, Some("project-ai"), Some("paper-ai"))
            .expect("project intersection")
            .len(),
        1
    );
    assert!(
        list_candidates(&connection, Some("missing-project"), Some("paper-ai"))
            .expect("outside project")
            .is_empty()
    );
    assert!(
        list_candidates(&connection, None, Some("missing-paper"))
            .expect("missing paper")
            .is_empty()
    );

    connection
        .execute(
            "update ai_evidence_items set paper_id='other-paper' where bundle_id=?1 and ordinal=1",
            [&bundle.id],
        )
        .expect("mixed paper sources");
    assert!(
        list_candidates(&connection, None, Some("paper-ai"))
            .expect("exclude mixed sources")
            .is_empty()
    );
}

#[test]
fn list_candidates_skips_unloadable_candidates() {
    let mut connection = database();
    let (anchor_id, _) = insert_selection_source(&connection);
    let (run, bundle) = seed(&connection, &anchor_id);
    let valid = save_generated_candidate(
        &mut connection,
        &run.id,
        &bundle,
        &selection_payload("可读取的候选。").to_string(),
    )
    .expect("valid candidate");

    connection
        .execute_batch(
            "pragma foreign_keys = off;
             insert into ai_runs
             (id, task_kind, status, provider_id, model_id, reasoning_effort,
              pipeline_version, prompt_version, output_schema_version, user_goal,
              scope_hash, input_item_count, max_input_tokens, max_output_tokens,
              timeout_ms, cancel_requested, created_at, updated_at)
             values ('bad-run', 'selection_explanation', 'needs_review',
                     'fixture-generation', 'fixture-model', null,
                     'selection-runtime-v1', 'selection-quote-first-v1', 1, '',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     1, 1000, 500, 5000, 0, 'later', 'later');
             insert into ai_evidence_items
             (id, bundle_id, citation_key, ordinal, anchor_id, paper_id, paper_title,
              attachment_id, content_sha256, page_index, page_label, quote_exact,
              quote_prefix, quote_suffix, context_text, relation_kind, validation_status,
              created_at)
             values ('bad-evidence', 'missing-bundle', 'BAD-1', 0, 'missing-anchor',
                     'paper-ai', 'Grounded Paper', 'attachment-ai',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     0, null, 'quote', '', '', 'quote', 'direct_quote', 'valid', 'later');
             insert into ai_candidates
             (id, run_id, evidence_bundle_id, candidate_kind, schema_version,
              current_revision, review_state, validation_status, original_payload_hash,
              created_at, updated_at)
             values ('bad-candidate', 'bad-run', 'missing-bundle',
                     'selection_explanation_v1', 1, 1, 'unreviewed', 'valid',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'later', 'later');
             insert into ai_candidate_revisions
             (id, candidate_id, revision, payload_json, payload_hash, editor_kind,
              source_run_id, parent_revision, change_reason, created_at)
             values ('bad-revision', 'bad-candidate', 1, '{}',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'ai', 'bad-run', null, 'provider_generation', 'later');
             pragma foreign_keys = on;",
        )
        .expect("corrupt candidate fixture");

    assert!(get_candidate(&connection, "bad-candidate").is_err());
    let listed = list_candidates(&connection, None, None).expect("candidate list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, valid.id);
}
