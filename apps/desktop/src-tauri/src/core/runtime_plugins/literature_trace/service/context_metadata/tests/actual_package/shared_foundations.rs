//! Real Core and exact independent package acceptance for the original foundations/run chain.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::algorithms::ConceptAlgorithmRun;

async fn fixture() -> Fixture {
    let mut f = Fixture::new();
    super::super::evolution_fixture::seed(&f);
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    f
}
async fn analyze(f: &Fixture) -> NativeResult<ConceptAlgorithmRun> {
    let binding = f.store.native_note_trace().unwrap();
    f.service
        .shared_foundations(f.store.clone(), binding.package_key.clone(), research_workbench_native_sdk::knowledge_promotion::algorithms::SharedFoundationsDraft {paper_ids: vec!["paper-1".into(), "paper-2".into(), "paper-3".into()]})
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
            "select count(*) from paper_foundation_similarity_results",
            [],
            |row| row.get(0),
        )
        .unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_foundations_late_result_failure_rolls_back_and_marks_existing_run_failed() {
    let f = fixture().await;
    f.store.connection().unwrap().execute_batch("create temp trigger fail_second_evolution before insert on paper_foundation_similarity_results when new.ordinal=1 begin select raise(abort,'late result failure'); end;").unwrap();
    let error = analyze(&f).await.unwrap_err();
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
    assert!(failed.foundation_similarities.is_empty());
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(get(&f, &id).await.unwrap(), failed);
    let retried = analyze(&f).await.unwrap();
    assert_ne!(retried.id, id);
    assert_eq!(retried.result_count, 3);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_foundations_finish_failure_preserves_original_committed_results_and_running_row()
 {
    let f = fixture().await;
    f.store.connection().unwrap().execute_batch("create temp trigger fail_evolution_finish before update on concept_algorithm_runs when new.status='succeeded' begin select raise(abort,'finish failure'); end;").unwrap();
    let error = analyze(&f).await.unwrap_err();
    let detail = error.concept_algorithm.unwrap();
    assert_eq!(detail.phase, "finish");
    let id = detail.object_id.unwrap();
    assert_eq!(rows(&f), (1, 3));
    let run = get(&f, &id).await.unwrap();
    assert_eq!(run.status, "running");
    assert_eq!(run.result_count, 0);
    assert_eq!(run.foundation_similarities.len(), 3);
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(get(&f, &id).await.unwrap(), run);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_foundations_unknown_result_commit_fences_and_reopens_original_run() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = fixture().await;
    let commits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&commits);
    f.store
        .connection()
        .unwrap()
        .commit_hook(Some(move || counter.fetch_add(1, Ordering::SeqCst) == 1));
    let error = analyze(&f).await.unwrap_err();
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
async fn actual_package_foundations_source_read_and_postcommit_read_failures_keep_original_stages()
{
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
        let error = analyze(&f).await.unwrap_err();
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
        assert_eq!(rows(&f), if postcommit { (1, 3) } else { (1, 0) });
        let f = reopen_actual_fixture(f, false).await;
        assert_eq!(get(&f, &id).await.unwrap(), run);
        evidence_matrix::close(f).await;
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_foundations_autocommit_failures_retain_known_run_and_actual_partial_writes()
{
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
        let error = analyze(&f).await.unwrap_err();
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
        assert_eq!(rows(&f), if start { (1, 0) } else { (1, 3) });
        let run = get(&f, &id).await.unwrap();
        assert_eq!(run.status, if start { "running" } else { "succeeded" });
        let f = reopen_actual_fixture(f, false).await;
        assert_eq!(get(&f, &id).await.unwrap(), run);
        evidence_matrix::close(f).await;
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_foundations_original_ranking_snapshots_scope_and_reopen() {
    let f = fixture().await;
    let run = analyze(&f).await.unwrap();
    assert_eq!(run.status, "succeeded");
    assert_eq!(run.algorithm_id, "foundational-concept-weighted-jaccard");
    assert_eq!(run.algorithm_version, 1);
    assert_eq!(run.concept_revision_watermark, 1);
    assert_eq!(run.relation_revision_watermark, 0);
    assert_eq!(run.result_count, 3);
    assert!(run.evolution_events.is_empty());
    let pairs = run
        .foundation_similarities
        .iter()
        .map(|row| {
            (
                row.left_paper_id.as_str(),
                row.right_paper_id.as_str(),
                row.weighted_jaccard,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        [
            ("paper-1", "paper-2", 0.5),
            ("paper-1", "paper-3", 0.5),
            ("paper-2", "paper-3", 0.0)
        ]
    );
    assert_eq!(
        run.foundation_similarities[0].shared_concepts[0].label,
        "Shared foundation"
    );
    assert_eq!(
        run.foundation_similarities[0].shared_concepts[0].weight,
        1.0 / (1.0 + 2_f64.ln())
    );
    for (ordinal, row) in run.foundation_similarities.iter().enumerate() {
        assert_eq!(row.ordinal, ordinal as i64);
    }
    let binding = f.store.native_note_trace().unwrap();
    let duplicate_scope = f.service.shared_foundations(f.store.clone(), binding.package_key.clone(), research_workbench_native_sdk::knowledge_promotion::algorithms::SharedFoundationsDraft {paper_ids: vec![" paper-3 ".into(), "paper-1".into(), "paper-2".into(), "paper-1".into()]}).await.unwrap();
    assert_eq!(
        duplicate_scope.foundation_similarities,
        run.foundation_similarities
    );
    assert_eq!(duplicate_scope.paper_scope_hash, run.paper_scope_hash);
    assert_eq!(duplicate_scope.parameters_hash, run.parameters_hash);
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(get(&f, &run.id).await.unwrap(), run);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_foundations_original_scope_error_priority_and_fifty_empty_papers() {
    use research_workbench_native_sdk::knowledge_promotion::algorithms::SharedFoundationsDraft;
    let f = fixture().await;
    let binding = f.store.native_note_trace().unwrap();
    for (ids, code, message) in [
        (
            vec![" ".into()],
            "invalid_input",
            "共享理论基础分析包含无效论文 ID",
        ),
        (
            vec!["paper-1".into(), " paper-1 ".into()],
            "invalid_input",
            "共享理论基础分析必须选择 2 至 50 篇不同论文",
        ),
        (vec!["paper-1".into(), "missing".into()], "not_found", ""),
        (
            (0..51).map(|i| format!("missing-{i}")).collect(),
            "invalid_input",
            "共享理论基础分析必须选择 2 至 50 篇不同论文",
        ),
    ] {
        let error = f
            .service
            .shared_foundations(
                f.store.clone(),
                binding.package_key.clone(),
                SharedFoundationsDraft { paper_ids: ids },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.message.contains(message));
    }
    assert_eq!(rows(&f), (0, 0));
    let ids = (0..50).map(|i| format!("empty-{i:02}")).collect::<Vec<_>>();
    {
        let db = f.store.connection().unwrap();
        for id in &ids {
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle',?1,'now','now')",[id]).unwrap();
        }
    }
    let run = f
        .service
        .shared_foundations(
            f.store.clone(),
            binding.package_key.clone(),
            SharedFoundationsDraft { paper_ids: ids },
        )
        .await
        .unwrap();
    assert_eq!(run.result_count, 1225);
    assert!(
        run.foundation_similarities
            .iter()
            .all(|row| row.weighted_jaccard == 0.0 && row.shared_concepts.is_empty())
    );
    assert_eq!(get(&f, &run.id).await.unwrap(), run);
    evidence_matrix::close(f).await;
}
