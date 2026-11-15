//! Shared Agent execution, original review semantics, atomic promotion and normal reopen.
use super::*;
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_executes_reviews_promotes_replays_and_reopens() {
    for edited in [false, true] {
        let (mut f, mut candidate) = generated().await;
        assert_eq!(candidate.candidate_kind, "derived_research_view_v1");
        assert_eq!(candidate.evidence_bundle.items.len(), 2);
        crate::ai::review_support::ReviewFacts::new(&f.store.connection().unwrap())
            .require_completed_turn(&candidate.id)
            .unwrap();
        if edited {
            let (mut value, _) =
                crate::ai::review_support::current_payload_source(&candidate).unwrap();
            value["items"][0]["text"] = json!("Human comparison Ω");
            candidate = f
                .service
                .revise_analysis_candidate(
                    f.store.clone(),
                    f.key.clone(),
                    AiCandidateRevisionDraft {
                        candidate_id: candidate.id.clone(),
                        expected_revision: 1,
                        payload: value,
                        change_reason: "Human source correction".into(),
                    },
                )
                .await
                .unwrap();
        }
        candidate = qualify(&f, &candidate).await;
        let request = review(
            &candidate,
            if edited {
                "accept_after_edit"
            } else {
                "accept_unchanged"
            },
        );
        let accepted = f
            .service
            .decide_analysis_candidate(f.store.clone(), f.key.clone(), request.clone())
            .await
            .unwrap();
        assert_eq!(
            accepted.decisions[0].target_object_kind.as_deref(),
            Some("derived_research_view")
        );
        assert_eq!(accepted.current_revision, if edited { 2 } else { 1 });
        let row: (String, String, String, i64, String) = f.store.connection().unwrap().query_row("select review_state,provenance_kind,inputs_json,current_revision,items_json from derived_research_views", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
        assert_eq!(row.0, "accepted");
        assert_eq!(
            row.1,
            if edited {
                "ai_accepted_after_edit"
            } else {
                "ai_suggestion"
            }
        );
        assert_eq!(row.3, 1);
        assert!(row.4.contains(if edited {
            "Human comparison Ω"
        } else {
            "provisional comparison"
        }));
        let original = serde_json::to_value(&accepted).unwrap();
        let replay = f
            .service
            .decide_analysis_candidate(f.store.clone(), f.key.clone(), request.clone())
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(replay).unwrap(), original);
        let (views, revisions, decisions): (i64,i64,i64) = f.store.connection().unwrap().query_row("select (select count(*) from derived_research_views),(select count(*) from derived_research_view_revisions),(select count(*) from ai_review_decisions)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!((views, revisions, decisions), (1, 1, 1));
        f = reopen_actual_fixture(f, false).await;
        let after = f
            .service
            .decide_analysis_candidate(f.store.clone(), f.key.clone(), request)
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), original);
        let bundle = f
            .service
            .persist_matrix_task_evidence(f.store.clone(), f.key.clone(), accepted.run_id)
            .await
            .unwrap();
        assert_eq!(bundle, candidate.evidence_bundle);
        super::super::derived_views::edit_accepted(&f).await;
        let edited_views = super::super::derived_views::list(&f).await;
        f = reopen_actual_fixture(f, false).await;
        assert_eq!(super::super::derived_views::list(&f).await, edited_views);
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_reject_and_defer_preserve_frozen_candidate_without_promotion()
 {
    for action in ["reject", "defer"] {
        let (f, candidate) = generated().await;
        let result = f
            .service
            .decide_analysis_candidate(f.store.clone(), f.key.clone(), review(&candidate, action))
            .await
            .unwrap();
        assert_eq!(
            result.review_state,
            if action == "reject" {
                "rejected"
            } else {
                "deferred"
            }
        );
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from derived_research_views", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(result.evidence_bundle, candidate.evidence_bundle);
        close(f).await;
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_provider_failure_and_cancel_keep_original_terminal_state()
{
    use std::sync::atomic::Ordering;
    for canceled in [false, true] {
        let (f, credentials, draft) = setup().await;
        let run = admit(&f, &credentials, draft).await;
        let signal = CancellationSignal::default();
        let (host, wire) = task_generation_host::Host::new(
            &f,
            credentials,
            run.clone(),
            signal.clone(),
            payload(),
        );
        let error = if canceled {
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            *wire.pause.lock().unwrap() = Some((started.clone(), release.clone()));
            let work = execute_research_task(&host, &run, &signal);
            let cancel = async {
                started.notified().await;
                let store = f.store.clone();
                assert!(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        tokio::task::spawn_blocking(move || store.connection().is_ok())
                    )
                    .await
                    .unwrap()
                    .unwrap()
                );
                signal.cancel();
                release.notify_one();
            };
            let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                tokio::join!(work, cancel)
            })
            .await
            .unwrap();
            result.unwrap_err()
        } else {
            wire.status.store(429, Ordering::SeqCst);
            execute_research_task(&host, &run, &signal)
                .await
                .unwrap_err()
        };
        assert_eq!(
            error.code(),
            if canceled { "canceled" } else { "rate_limited" }
        );
        assert!(!error.to_string().contains("synthetic private provider"));
        host.terminal_failure(&error).await.unwrap();
        let row:(String,i64)=f.store.connection().unwrap().query_row("select status,(select count(*) from ai_candidates where run_id=r.id) from ai_runs r where id=?1",[&run.id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(
            (row.0.as_str(), row.1),
            (if canceled { "canceled" } else { "failed" }, 0)
        );
        drop(host);
        close(f).await;
    }
}
