//! Controlled finite replies exercise Core's original transaction reentry and ownership.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::{candidate::CandidateInputV1, wire::*},
    candidate_review::{generic::GenericPromotionInputV1, wire::*},
    catalog::CatalogOutput,
    paper_context_wire::encode_payload,
};
fn fixture(wrong: bool) -> (Fixture, AiReviewDecisionDraft) {
    let mut f = Fixture::new();
    let mut candidate = seed(&f, payload(true));
    let value = payload(true);
    let decoded = protocol::decode_candidate_value(
        value.clone(),
        &std::collections::HashSet::from(["ev-quote".into()]),
    )
    .unwrap();
    for reference in decoded.citation_references() {
        candidate = crate::ai::review_support::record_citation_assessment_with(
            &f.store.connection().unwrap(),
            AiCitationAssessmentDraft {
                candidate_id: candidate.id.clone(),
                candidate_revision: 1,
                statement_id: reference.statement_id.clone(),
                evidence_item_id: "legacy-item".into(),
                expected_assessment_revision: 0,
                semantic_relation: "supports".into(),
                rationale: "Synthetic checked source".into(),
                idempotency_key: reference.statement_id,
            },
            |_| Ok::<_, AiError>(decoded.clone()),
        )
        .unwrap();
    }
    f.store.connection().unwrap().execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values('inspected','legacy-candidate',1,'legacy-item','now')",[]).unwrap();
    let request = draft(&candidate, "accept_unchanged");
    let input = GenericPromotionInputV1 {
        payload: value.clone(),
        reading_unit_title: request.reading_unit_title.clone(),
        project_id: None,
        paper_id: "paper".into(),
        anchor_ids: vec!["legacy-anchor".into()],
        provenance_kind: "ai_accepted_unchanged".into(),
        candidate_id: candidate.id.clone(),
        candidate_revision: 1,
    };
    let research_workbench_native_sdk::candidate_review::generic::GenericPromotionPlanV1::ReadingUnits { units } = protocol::generic_prepare(&input).unwrap() else { panic!("units"); };
    let review = |input| WorkflowInputV1::CandidateReview {
        input: Box::new(input),
    };
    let mut inputs = vec![
        review(ReviewInputV1::ValidateDraft {
            draft: request.clone(),
        }),
        review(ReviewInputV1::Candidate {
            draft: request.clone(),
            observation: candidate.review_observation(1),
        }),
        review(ReviewInputV1::Inspection { inspected: true }),
        WorkflowInputV1::Candidate {
            input: Box::new(CandidateInputV1::Decode {
                content: value.to_string(),
                evidence_keys: vec!["ev-quote".into()],
            }),
        },
        review(ReviewInputV1::GenericPromotion {
            input: Box::new(input),
        }),
    ];
    for (ordinal, draft) in units.into_iter().enumerate() {
        inputs.push(review(ReviewInputV1::ReadingUnitValidation {
            ordinal,
            draft: Box::new(draft),
        }));
    }
    let outputs = inputs
        .into_iter()
        .enumerate()
        .map(|(ordinal, input)| {
            let decision = match &input {
                WorkflowInputV1::CandidateReview { input } => WorkflowDecisionV1::CandidateReview {
                    decision: protocol::review(*input.clone()),
                },
                WorkflowInputV1::Candidate { input } => WorkflowDecisionV1::Candidate {
                    decision: protocol::candidate(*input.clone()),
                },
                _ => unreachable!(),
            };
            CatalogOutput::AnalysisWorkflow {
                outcome: Box::new(WorkflowWireOutcomeV1 {
                    input: WorkflowWireInputV1 {
                        cbor_base64: encode_payload(&input).unwrap(),
                    },
                    decision_cbor_base64: encode_payload(&if wrong && ordinal == 6 {
                        WorkflowDecisionV1::Validated { rejection: None }
                    } else {
                        decision
                    })
                    .unwrap(),
                }),
            }
        })
        .collect::<Vec<_>>();
    let directory = f.package.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.analysis-workflow","version":1}));
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in [
        "reading-unit-accepted-insert.v1.sql",
        "reading-unit-accepted-evidence.v1.sql",
        "nal-bootstrap-research-revision-insert.v1.sql",
    ] {
        fs::copy(source.join(name), directory.join("data").join(name)).unwrap();
    }
    f.configure(&outputs, Some(6), false);
    (f, request)
}
#[tokio::test]
async fn generic_reading_second_field_wait_releases_guard_rolls_back_prefix_and_rechecks_source() {
    let (f, request) = fixture(false);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let call =
        tokio::spawn(async move { owner.decide_analysis_candidate(store, key, request).await });
    assert!(f.entered(6).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(
            db.query_row("select count(*) from structured_reading_units", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        db.execute(
            "update ai_candidates set review_state='rejected' where id='legacy-candidate'",
            [],
        )
        .unwrap();
    });
    let available = tokio::time::timeout(super::super::super::WAIT, &mut writer).await;
    f.release();
    let error = call.await.unwrap().unwrap_err();
    if available.is_err() {
        writer.await.unwrap();
    }
    assert!(available.is_ok());
    assert!(error.to_string().contains("plugin_host_source_changed"));
    assert_eq!(counts(&f), (0, 0, 0, 0));
    close(f).await;
}
#[tokio::test]
async fn generic_reading_wrong_later_validation_family_keeps_all_writes_absent() {
    let (f, request) = fixture(true);
    f.release();
    let error = decide(&f, request).await.unwrap_err();
    assert!(error.to_string().contains("plugin_host_response_mismatch"));
    assert_eq!(counts(&f), (0, 0, 0, 0));
    assert!(f.store.connection().unwrap().is_autocommit());
    close(f).await;
}
#[tokio::test]
async fn generic_reading_caller_exit_retains_original_owned_completion() {
    let (f, request) = fixture(false);
    let replay = request.clone();
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let call =
        tokio::spawn(async move { owner.decide_analysis_candidate(store, key, request).await });
    assert!(f.entered(6).await);
    call.abort();
    f.release();
    tokio::time::timeout(super::super::super::WAIT, async {
        while counts(&f).3 == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(counts(&f), (2, 2, 2, 1));
    // The original durable record is present before the owner can shut down.
    assert!(
        ReviewFacts::new(&f.store.connection().unwrap())
            .generic_decision(&replay.idempotency_key)
            .unwrap()
            .is_some()
    );
    close(f).await;
}
