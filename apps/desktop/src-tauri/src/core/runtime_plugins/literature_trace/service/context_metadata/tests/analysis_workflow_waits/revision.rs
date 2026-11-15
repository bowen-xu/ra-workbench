//! Revision preparation reuses original candidate facts without holding Core's guard.
use super::*;
use crate::ai::{
    generation_policy::CandidateValidationFacts,
    review_support::{ReviewFacts, current_payload_source},
};
use research_workbench_native_sdk::{
    analysis_workflow::candidate::{CandidateDecisionV1, CandidateInputV1},
    candidate_review::contracts::AiCandidateRevisionDraft,
};

fn fixture(wrong: bool) -> (Fixture, AiCandidateRevisionDraft) {
    let (mut f, citation) = citation::fixture(false);
    let (request, inputs) = {
        let db = f.store.connection().unwrap();
        let candidate = ReviewFacts::new(&db)
            .candidate(&citation.candidate_id)
            .unwrap();
        let (mut value, keys) = current_payload_source(&candidate).unwrap();
        value["fields"][0]["value"] = json!("Controlled human revision Ω");
        let decode = CandidateInputV1::Decode {
            content: value.to_string(),
            evidence_keys: keys,
        };
        let CandidateDecisionV1::Decoded {
            payload_cbor_base64,
        } = research_literature_library_native::analysis_workflow::candidate::decide(
            decode.clone(),
        )
        .unwrap()
        else {
            panic!("expected original decode")
        };
        let facts = CandidateValidationFacts::new(&db);
        let inputs = vec![
            decode,
            CandidateInputV1::RevisionQuotesAndKind {
                payload_cbor_base64: payload_cbor_base64.clone(),
                candidate_kind: candidate.candidate_kind,
                evidence: candidate
                    .evidence_bundle
                    .items
                    .iter()
                    .map(
                        |item| research_workbench_native_sdk::candidate_contracts::EvidenceQuote {
                            citation_key: item.citation_key.clone(),
                            quote_exact: item.quote_exact.clone(),
                        },
                    )
                    .collect(),
            },
            CandidateInputV1::Profile {
                payload_cbor_base64: payload_cbor_base64.clone(),
                profile: facts.overview_profile(&candidate.run_id).unwrap(),
            },
            CandidateInputV1::RevisionMatrix {
                payload_cbor_base64: payload_cbor_base64.clone(),
                matrix: facts.matrix_context(&candidate.run_id).unwrap(),
            },
            CandidateInputV1::RevisionConfirmedFields {
                payload_cbor_base64,
                active_fields: vec![],
            },
        ];
        (
            AiCandidateRevisionDraft {
                candidate_id: candidate.id,
                expected_revision: 1,
                payload: value,
                change_reason: "Controlled wait".into(),
            },
            inputs,
        )
    };
    let outputs = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            let decision =
                research_literature_library_native::analysis_workflow::candidate::decide(
                    input.clone(),
                )
                .unwrap();
            let response = if wrong && index == 2 {
                WorkflowDecisionV1::Validated { rejection: None }
            } else {
                WorkflowDecisionV1::Candidate { decision }
            };
            CatalogOutput::AnalysisWorkflow {
                outcome: Box::new(WorkflowWireOutcomeV1 {
                    input: WorkflowWireInputV1 {
                        cbor_base64: encode_payload(&WorkflowInputV1::Candidate {
                            input: Box::new(input),
                        })
                        .unwrap(),
                    },
                    decision_cbor_base64: encode_payload(&response).unwrap(),
                }),
            }
        })
        .collect::<Vec<_>>();
    f.configure(&outputs, Some(0), false);
    (f, request)
}
fn revisions(f: &Fixture, id: &str) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "select count(*) from ai_candidate_revisions where candidate_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn revision_wait_releases_guard_and_rechecks_original_terminal_review() {
    let (f, request) = fixture(false);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let id = request.candidate_id.clone();
    let call =
        tokio::spawn(async move { owner.revise_analysis_candidate(store, key, request).await });
    assert!(f.entered(0).await);
    let store = f.store.clone();
    let candidate = id.clone();
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
    assert!(
        released.is_ok(),
        "Core guard must be released during revision Native wait"
    );
    assert!(error.to_string().contains("已结束审查的候选不能再修改"));
    assert_eq!(revisions(&f, &id), 1);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn revision_late_wrong_native_response_writes_nothing_and_keeps_request_identity() {
    let (f, request) = fixture(true);
    let id = request.candidate_id.clone();
    f.release();
    let error = f
        .service
        .revise_analysis_candidate(f.store.clone(), f.key.clone(), request)
        .await
        .unwrap_err();
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error else {
        panic!("expected Native diagnostic")
    };
    assert_eq!(failure.code, "plugin_host_response_mismatch");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    assert_eq!(revisions(&f, &id), 1);
    assert!(f.store.connection().unwrap().is_autocommit());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn accepted_revision_survives_caller_loss_and_drains_on_shutdown() {
    for close in [false, true] {
        let (f, request) = fixture(false);
        let id = request.candidate_id.clone();
        let owner = f.service.clone();
        let store = f.store.clone();
        let key = f.key.clone();
        let call =
            tokio::spawn(async move { owner.revise_analysis_candidate(store, key, request).await });
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
        assert_eq!(revisions(&f, &id), if close { 1 } else { 2 });
        assert!(f.store.connection().unwrap().is_autocommit());
    }
}
