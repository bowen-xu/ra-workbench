//! Generic review has guard-free waits and preserves authoritative idempotent replay.
use super::*;
use crate::ai::review_support::{ReviewFacts, current_payload_source};
use research_workbench_native_sdk::{
    analysis_workflow::candidate::CandidateInputV1,
    candidate_review::{contracts::AiReviewDecisionDraft, wire::ReviewInputV1},
};
fn output(input: WorkflowInputV1, wrong: bool) -> CatalogOutput {
    let decision = match &input {
        WorkflowInputV1::CandidateReview { input } => WorkflowDecisionV1::CandidateReview {
            decision: research_literature_library_native::candidate_review::decide(*input.clone()),
        },
        WorkflowInputV1::Candidate { input } => WorkflowDecisionV1::Candidate {
            decision: research_literature_library_native::analysis_workflow::candidate::decide(
                *input.clone(),
            )
            .unwrap(),
        },
        _ => unreachable!(),
    };
    CatalogOutput::AnalysisWorkflow {
        outcome: Box::new(WorkflowWireOutcomeV1 {
            input: WorkflowWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            },
            decision_cbor_base64: encode_payload(&if wrong {
                WorkflowDecisionV1::Validated { rejection: None }
            } else {
                decision
            })
            .unwrap(),
        }),
    }
}
fn fixture(replay: bool, wrong: bool, wait: usize) -> (Fixture, AiReviewDecisionDraft) {
    let (mut f, citation) = citation::fixture(false);
    let candidate = ReviewFacts::new(&f.store.connection().unwrap())
        .candidate(&citation.candidate_id)
        .unwrap();
    let request = AiReviewDecisionDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: 1,
        action: "defer".into(),
        rationale: "Synthetic review".into(),
        idempotency_key: "controlled-review".into(),
        reading_unit_title: None,
        project_id: None,
    };
    let mut inputs = vec![
        WorkflowInputV1::CandidateReview {
            input: Box::new(ReviewInputV1::ValidateDraft {
                draft: request.clone(),
            }),
        },
        WorkflowInputV1::CandidateReview {
            input: Box::new(ReviewInputV1::Candidate {
                draft: request.clone(),
                observation: candidate.review_observation(1),
            }),
        },
    ];
    if replay {
        inputs.push(WorkflowInputV1::CandidateReview {
            input: Box::new(ReviewInputV1::Replay {
                draft: request.clone(),
                existing: (candidate.id, 1, "defer".into(), "Synthetic review".into()),
            }),
        });
    } else {
        let (value, keys) = current_payload_source(&candidate).unwrap();
        inputs.push(WorkflowInputV1::Candidate {
            input: Box::new(CandidateInputV1::Decode {
                content: value.to_string(),
                evidence_keys: keys,
            }),
        });
    }
    let outputs = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| output(input, wrong && index == 2))
        .collect::<Vec<_>>();
    f.configure(&outputs, Some(wait), false);
    (f, request)
}
fn count(f: &Fixture) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row("select count(*) from ai_review_decisions", [], |row| {
            row.get(0)
        })
        .unwrap()
}
#[tokio::test]
async fn review_wait_releases_guard_and_rechecks_changed_observation() {
    let (f, request) = fixture(false, false, 1);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let candidate = request.candidate_id.clone();
    let call =
        tokio::spawn(async move { owner.decide_analysis_candidate(store, key, request).await });
    assert!(f.entered(1).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update ai_candidates set review_state='rejected' where id=?1",
            [candidate],
        )
        .unwrap();
    });
    let released = tokio::time::timeout(WAIT, &mut writer).await;
    f.release();
    let error = tokio::time::timeout(WAIT, call)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(released.is_ok());
    assert!(error.to_string().contains("plugin_host_source_changed"));
    assert_eq!(count(&f), 0);
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn review_concurrent_same_key_follows_original_replay_instead_of_old_rule_slot() {
    let (f, request) = fixture(true, false, 1);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let concurrent = request.clone();
    let call =
        tokio::spawn(async move { owner.decide_analysis_candidate(store, key, request).await });
    assert!(f.entered(1).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        actual_package::defer_candidate(
            &mut store.connection().unwrap(),
            concurrent,
        )
        .unwrap()
    });
    let released = tokio::time::timeout(WAIT, &mut writer).await;
    f.release();
    let candidate = tokio::time::timeout(WAIT, call)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(released.is_ok());
    assert_eq!(candidate.review_state, "deferred");
    assert_eq!(count(&f), 1);
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn review_wrong_late_result_has_identity_and_writes_nothing() {
    let (f, request) = fixture(false, true, 2);
    f.release();
    let error = f
        .service
        .decide_analysis_candidate(f.store.clone(), f.key.clone(), request)
        .await
        .unwrap_err();
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error else {
        panic!("expected Native diagnostic")
    };
    assert_eq!(failure.code, "plugin_host_response_mismatch");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    assert_eq!(count(&f), 0);
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn accepted_review_survives_caller_loss_and_drains_during_shutdown() {
    for close in [false, true] {
        let (f, request) = fixture(false, false, 0);
        let owner = f.service.clone();
        let store = f.store.clone();
        let key = f.key.clone();
        let call =
            tokio::spawn(async move { owner.decide_analysis_candidate(store, key, request).await });
        assert!(f.entered(0).await);
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        assert!(f.service.doi_operations.try_write().is_err());
        if close {
            let stopped = f.service.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            let owner = f.service.clone();
            let shutdown = tokio::spawn(async move { owner.shutdown().await });
            tokio::time::timeout(WAIT, &mut stopped).await.unwrap();
            f.release();
            tokio::time::timeout(WAIT, shutdown)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        } else {
            f.release();
            let drained = tokio::time::timeout(WAIT, f.service.doi_operations.write())
                .await
                .unwrap();
            drop(drained);
            f.service.shutdown().await.unwrap();
        }
        assert_eq!(count(&f), if close { 0 } else { 1 });
        assert!(f.store.connection().unwrap().is_autocommit());
    }
}
