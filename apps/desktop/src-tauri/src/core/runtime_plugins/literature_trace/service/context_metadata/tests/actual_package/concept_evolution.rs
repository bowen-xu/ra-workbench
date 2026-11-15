//! Real Core and exact independent package acceptance for the original evolution/run chain.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::algorithms::ConceptAlgorithmRun;

async fn fixture() -> Fixture {
    let mut f = Fixture::new();
    super::super::evolution_fixture::seed(&f);
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    f
}
async fn analyze(f: &Fixture, id: &str) -> NativeResult<ConceptAlgorithmRun> {
    let binding = f.store.native_note_trace().unwrap();
    f.service
        .concept_evolution(f.store.clone(), binding.package_key.clone(), id.into())
        .await
}
async fn get(f: &Fixture, id: &str) -> NativeResult<ConceptAlgorithmRun> {
    let binding = f.store.native_note_trace().unwrap();
    f.service
        .concept_algorithm_run(f.store.clone(), binding.package_key.clone(), id.into())
        .await
}
fn rows(f: &Fixture) -> (i64, i64) {
    let db = f.store.connection().unwrap();
    (
        db.query_row("select count(*) from concept_algorithm_runs", [], |row| {
            row.get(0)
        })
        .unwrap(),
        db.query_row(
            "select count(*) from concept_evolution_results",
            [],
            |row| row.get(0),
        )
        .unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_original_order_snapshots_get_and_reopen() {
    let f = fixture().await;
    let run = analyze(&f, " concept-shared ").await.unwrap();
    assert_eq!(run.status, "succeeded");
    assert_eq!(run.algorithm_id, "accepted-paper-concept-timeline");
    assert_eq!(run.algorithm_version, 1);
    assert_eq!(run.result_count, 2);
    assert_eq!(run.evolution_events[0].paper_id, "paper-1");
    assert_eq!(run.evolution_events[0].event_kind, "introduced");
    assert_eq!(run.evolution_events[1].paper_id, "paper-2");
    assert_eq!(run.evolution_events[1].event_kind, "adapted");
    assert_eq!(run.concept_revision_watermark, 1);
    assert_eq!(run.relation_revision_watermark, 0);
    assert_eq!(rows(&f), (1, 2));
    assert_eq!(get(&f, &format!(" {} ", run.id)).await.unwrap(), run);
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(get(&f, &run.id).await.unwrap(), run);
    assert_eq!(rows(&f), (1, 2));
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_errors_empty_runs_and_existing_foundation_results() {
    use crate::runtime_plugins::literature_knowledge_promotion::{KnowledgeSql, algorithm_runs};
    let f = fixture().await;
    for (id, code) in [(" ", "invalid_input"), ("missing", "not_found")] {
        let error = analyze(&f, id).await.unwrap_err();
        assert_eq!(error.code, code);
        if code == "invalid_input" {
            assert!(error.request_id.is_some());
        }
    }
    assert_eq!(
        analyze(&f, &"Ω".repeat(81)).await.unwrap_err().code,
        "invalid_input"
    );
    assert_eq!(
        get(&f, &" ".repeat(161)).await.unwrap_err().code,
        "invalid_input"
    );
    assert_eq!(rows(&f), (0, 0));
    f.store.connection().unwrap().execute_batch("insert into concepts(id,label,explanation,created_at,updated_at) values('empty','Empty','','now','now');").unwrap();
    let empty = analyze(&f, "empty").await.unwrap();
    assert_eq!(empty.result_count, 0);
    assert!(empty.evolution_events.is_empty());
    assert_eq!(empty.status, "succeeded");
    let id = {
        let db = f.store.connection().unwrap();
        let sql = KnowledgeSql::for_test(
            &research_literature_library_native::knowledge_promotion::storage::static_sql(),
        );
        let run = algorithm_runs::start(
            &db,
            &sql,
            "shared_foundations",
            "foundational-concept-weighted-jaccard",
            1,
            &json!({"paperCount":2}),
            &["paper-1".into(), "paper-2".into()],
        )
        .unwrap();
        db.execute("insert into paper_foundation_similarity_results(run_id,ordinal,left_paper_id,left_paper_title,right_paper_id,right_paper_title,shared_concepts_json,weighted_jaccard,shared_concept_count,left_foundation_count,right_foundation_count) values(?1,0,'paper-1','Left','paper-2','Right','[]',0,0,0,0)",[&run.id]).unwrap();
        algorithm_runs::finish(&db, &sql, &run.id, 1).unwrap();
        run.id
    };
    let run = get(&f, &id).await.unwrap();
    assert_eq!(run.algorithm_kind, "shared_foundations");
    assert_eq!(run.foundation_similarities.len(), 1);
    assert!(run.evolution_events.is_empty());
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(get(&f, &id).await.unwrap(), run);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_late_result_failure_rolls_back_and_marks_existing_run_failed() {
    let f = fixture().await;
    f.store.connection().unwrap().execute_batch("create temp trigger fail_second_evolution before insert on concept_evolution_results when new.ordinal=1 begin select raise(abort,'late result failure'); end;").unwrap();
    let error = analyze(&f, "concept-shared").await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    let detail = error.concept_algorithm.unwrap();
    assert_eq!(
        detail.outcome,
        crate::runtime_plugins::native::StorageMutationOutcome::RolledBack
    );
    assert!(detail.database_error.is_some());
    let id = detail.object_id.unwrap();
    assert_eq!(rows(&f), (1, 0));
    let failed = get(&f, &id).await.unwrap();
    assert_eq!(failed.status, "failed");
    assert!(failed.evolution_events.is_empty());
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(get(&f, &id).await.unwrap(), failed);
    let retried = analyze(&f, "concept-shared").await.unwrap();
    assert_ne!(retried.id, id);
    assert_eq!(retried.result_count, 2);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_finish_failure_preserves_original_committed_results_and_running_row()
 {
    let f = fixture().await;
    f.store.connection().unwrap().execute_batch("create temp trigger fail_evolution_finish before update on concept_algorithm_runs when new.status='succeeded' begin select raise(abort,'finish failure'); end;").unwrap();
    let error = analyze(&f, "concept-shared").await.unwrap_err();
    let detail = error.concept_algorithm.unwrap();
    assert_eq!(detail.phase, "finish");
    let id = detail.object_id.unwrap();
    assert_eq!(rows(&f), (1, 2));
    let run = get(&f, &id).await.unwrap();
    assert_eq!(run.status, "running");
    assert_eq!(run.result_count, 0);
    assert_eq!(run.evolution_events.len(), 2);
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(get(&f, &id).await.unwrap(), run);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_unknown_result_commit_fences_and_reopens_original_run() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = fixture().await;
    let commits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&commits);
    f.store
        .connection()
        .unwrap()
        .commit_hook(Some(move || counter.fetch_add(1, Ordering::SeqCst) == 1));
    let error = analyze(&f, "concept-shared").await.unwrap_err();
    assert_eq!(error.code, "data_operation_outcome_unknown");
    let detail = error.concept_algorithm.unwrap();
    assert_eq!(
        detail.outcome,
        crate::runtime_plugins::native::StorageMutationOutcome::Unknown
    );
    let id = detail.object_id.unwrap();
    assert!(f.store.connection().is_err());
    let f = reopen_actual_fixture(f, true).await;
    assert_eq!(rows(&f), (1, 0));
    assert_eq!(get(&f, &id).await.unwrap().status, "running");
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_chunks_preserve_current_versions_order_and_source_fallback() {
    let f = fixture().await;
    {
        let db = f.store.connection().unwrap();
        db.execute_batch("pragma foreign_keys=off;
            insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at) values
            ('evolution-anchor-1','paper-1','fixture-attachment','fixture-content',0,'Original source one','now'),
            ('evolution-anchor-3','paper-3','fixture-attachment','fixture-content',0,'Original source three','now');
            insert into terms(id,text,normalized_text,language,created_at,updated_at) values('evolution-term','Theory','theory','en','now','now');
            insert into term_usages(id,term_id,concept_id,anchor_id,usage_text,created_at,updated_at) values
            ('evolution-usage-1','evolution-term','concept-shared','evolution-anchor-1','source one','now','now'),
            ('evolution-usage-3','evolution-term','concept-shared','evolution-anchor-3','source three','now','now');
            insert into paper_concept_assertion_revisions
            (id,assertion_id,revision,paper_function,novelty_status,explanation_status,explanation,role_rationale,novelty_rationale,source_candidate_id,source_candidate_revision,source_local_concept_id,created_at)
            select 'assertion-1-revision-2',assertion_id,2,paper_function,'pre_existing',explanation_status,'Current explanation Ω',role_rationale,novelty_rationale,source_candidate_id,2,source_local_concept_id,created_at
            from paper_concept_assertion_revisions where id='assertion-1-revision';
            update paper_concept_assertions set current_revision=2 where id='assertion-1';
            update concepts set revision=3 where id='concept-shared';
            update concepts set revision=5 where id='concept-other';").unwrap();
        for index in 0..65 {
            super::super::evolution_fixture::insert_assertion(
                &db,
                &format!("batch-{index:03}"),
                "paper-1",
                "concept-shared",
                "focal",
                "unclear",
                "Chunk explanation Ω",
            );
        }
        super::super::evolution_fixture::insert_assertion(
            &db,
            "usage-assertion",
            "paper-3",
            "concept-shared",
            "prerequisite",
            "pre_existing",
            "Usage concept resolution",
        );
        db.execute_batch("update paper_concept_assertions set concept_id=null,term_usage_id='evolution-usage-3' where id='usage-assertion'; pragma foreign_keys=on;").unwrap();
    }
    let run = analyze(&f, "concept-shared").await.unwrap();
    assert_eq!(run.result_count, 68);
    assert_eq!(run.concept_revision_watermark, 5);
    let first = &run.evolution_events[0];
    assert_eq!(first.assertion_id, "assertion-1");
    assert_eq!(first.assertion_revision, 2);
    assert_eq!(first.concept_revision, 3);
    assert_eq!(first.explanation, "Current explanation Ω");
    assert_eq!(first.event_kind, "reused");
    assert_eq!(
        first.evidence_anchor_id.as_deref(),
        Some("evolution-anchor-1")
    );
    assert_eq!(run.evolution_events[65].assertion_id, "batch-064");
    assert_eq!(run.evolution_events[66].paper_id, "paper-3");
    assert_eq!(
        run.evolution_events[66].evidence_anchor_id.as_deref(),
        Some("evolution-anchor-3")
    );
    assert_eq!(run.evolution_events[67].paper_id, "paper-2");
    for (ordinal, event) in run.evolution_events.iter().enumerate() {
        assert_eq!(event.ordinal, ordinal as i64);
    }
    let repeated = analyze(&f, "concept-shared").await.unwrap();
    assert_ne!(repeated.id, run.id);
    assert_eq!(repeated.parameters_hash, run.parameters_hash);
    assert_eq!(repeated.paper_scope_hash, run.paper_scope_hash);
    assert_eq!(repeated.evolution_events, run.evolution_events);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_source_read_and_postcommit_read_failures_keep_original_stages() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    for postcommit in [false, true] {
        let f = fixture().await;
        let commits = Arc::new(AtomicUsize::new(0));
        {
            let db = f.store.connection().unwrap();
            let count = Arc::clone(&commits);
            db.commit_hook(Some(move || {
                count.fetch_add(1, Ordering::SeqCst);
                false
            }));
            let count = Arc::clone(&commits);
            db.authorizer(Some(move |context: AuthContext<'_>| {
                let deny = if postcommit {
                    count.load(Ordering::SeqCst) >= 3
                        && matches!(
                            context.action,
                            AuthAction::Read {
                                table_name: "concept_algorithm_runs",
                                ..
                            }
                        )
                } else {
                    matches!(
                        context.action,
                        AuthAction::Read {
                            table_name: "paper_concept_assertion_revisions",
                            ..
                        }
                    )
                };
                if deny {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }));
        }
        let error = analyze(&f, "concept-shared").await.unwrap_err();
        let detail = error.concept_algorithm.unwrap();
        let id = detail.object_id.unwrap();
        if postcommit {
            assert_eq!(detail.phase, "postcommit-read");
            assert_eq!(
                detail.outcome,
                crate::runtime_plugins::native::StorageMutationOutcome::Committed
            );
        }
        f.store
            .connection()
            .unwrap()
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        let run = get(&f, &id).await.unwrap();
        assert_eq!(run.status, if postcommit { "succeeded" } else { "failed" });
        assert_eq!(rows(&f), if postcommit { (1, 2) } else { (1, 0) });
        let f = reopen_actual_fixture(f, false).await;
        assert_eq!(get(&f, &id).await.unwrap(), run);
        evidence_matrix::close(f).await;
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evolution_autocommit_failures_retain_known_run_and_actual_partial_writes() {
    for start in [true, false] {
        let f = fixture().await;
        let trigger = if start {
            "create temp trigger evolution_insert_failed_after_row after insert on concept_algorithm_runs begin select raise(fail,'start row retained'); end;"
        } else {
            "create temp trigger evolution_finish_failed_after_update after update on concept_algorithm_runs when new.status='succeeded' begin select raise(fail,'finish update retained'); end;"
        };
        f.store
            .connection()
            .unwrap()
            .execute_batch(trigger)
            .unwrap();
        let error = analyze(&f, "concept-shared").await.unwrap_err();
        assert_eq!(error.code, "storage_error");
        let detail = error.concept_algorithm.unwrap();
        assert_eq!(detail.phase, if start { "start" } else { "finish" });
        assert_eq!(
            detail.outcome,
            crate::runtime_plugins::native::StorageMutationOutcome::StatementFailed
        );
        assert!(detail.database_error.is_some());
        let id = detail.object_id.unwrap();
        assert!(f.store.connection().unwrap().is_autocommit());
        assert_eq!(rows(&f), if start { (1, 0) } else { (1, 2) });
        let run = get(&f, &id).await.unwrap();
        assert_eq!(run.status, if start { "running" } else { "succeeded" });
        let f = reopen_actual_fixture(f, false).await;
        assert_eq!(get(&f, &id).await.unwrap(), run);
        evidence_matrix::close(f).await;
    }
}
