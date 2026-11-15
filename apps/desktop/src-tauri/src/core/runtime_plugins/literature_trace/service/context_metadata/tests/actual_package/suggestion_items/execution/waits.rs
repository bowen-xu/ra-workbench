//! Controlled replies use the real review driver and its original transaction owner.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::{candidate::CandidateInputV1, wire::*},
    candidate_review::items::{ItemReviewInputV1 as Input, SuggestionV1},
    catalog::CatalogOutput,
    concepts::NewConceptDetails,
    knowledge_promotion::KnowledgeValidationInputV1 as Check,
    paper_context_wire::encode_payload,
};
fn output(input: WorkflowInputV1, wrong: bool) -> CatalogOutput {
    let decision = match &input {
        WorkflowInputV1::SuggestionReview { input } => WorkflowDecisionV1::SuggestionReview {
            decision: research_literature_library_native::candidate_review::items::decide(
                *input.clone(),
            ),
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
fn fixture(wrong: bool) -> (Fixture, AiSuggestionItemDecisionDraft) {
    let mut f = Fixture::new();
    let mut value = payload("concept_suggestion_v1");
    value["suggestions"][0]["evidenceItemIds"] = json!(["ev-quote"]);
    let mut candidate = super::super::super::generic_reading::seed(&f, value.clone());
    let keys = std::collections::HashSet::from(["ev-quote".into()]);
    let decoded =
        research_literature_library_native::analysis_workflow::candidate::decode_candidate_value(
            value.clone(),
            &keys,
        )
        .unwrap();
    candidate = crate::ai::review_support::record_citation_assessment_with(
        &f.store.connection().unwrap(),
        AiCitationAssessmentDraft {
            candidate_id: candidate.id.clone(),
            candidate_revision: 1,
            statement_id: "concept_1".into(),
            evidence_item_id: "legacy-item".into(),
            expected_assessment_revision: 0,
            semantic_relation: "supports".into(),
            rationale: "Synthetic checked source".into(),
            idempotency_key: "qualification".into(),
        },
        |_| Ok::<_, crate::ai::error::AiError>(decoded.clone()),
    )
    .unwrap();
    f.store.connection().unwrap().execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values('inspection','legacy-candidate',1,'legacy-item','now')",[]).unwrap();
    let request = request(
        &candidate,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let selected = Box::new(SuggestionV1::Concept(
        serde_json::from_value(value["suggestions"][0].clone()).unwrap(),
    ));
    let anchors = vec!["legacy-anchor".into()];
    let review = |input| WorkflowInputV1::SuggestionReview {
        input: Box::new(input),
    };
    let mut inputs = vec![
        review(Input::Draft {
            draft: request.clone(),
        }),
        review(Input::Candidate {
            revision: 1,
            current_revision: 1,
            review_state: candidate.review_state,
        }),
        review(Input::Terminal {
            has_terminal_decision: false,
        }),
        WorkflowInputV1::Candidate {
            input: Box::new(CandidateInputV1::Decode {
                content: value.to_string(),
                evidence_keys: vec!["ev-quote".into()],
            }),
        },
        review(Input::Select {
            payload: value,
            suggestion_id: "concept_1".into(),
        }),
        review(Input::Action {
            draft: request.clone(),
            editor_kind: "ai".into(),
            suggestion: selected.clone(),
        }),
        review(Input::Inspection { inspected: true }),
        review(Input::Anchors {
            evidence_ids: vec!["ev-quote".into()],
            sources: vec![("ev-quote".into(), "legacy-anchor".into())],
        }),
        review(Input::Promotion {
            suggestion: selected,
            resolution: request.resolution.clone(),
            anchor_ids: anchors.clone(),
        }),
    ];
    for (ordinal, input) in [
        Check::Origin {
            provenance_kind: "ai_accepted_unchanged".into(),
            anchor_ids: anchors,
        },
        Check::UniqueAnchor { duplicate: false },
        Check::Term {
            text: "稀疏门控".into(),
            language: String::new(),
        },
        Check::Details {
            details: NewConceptDetails {
                label: "稀疏门控".into(),
                explanation: "每次路由只激活部分专家。".into(),
                disambiguation: String::new(),
                domain: String::new(),
                tradition: String::new(),
                period: String::new(),
                change_reason: Some("ai_suggestion_acceptance".into()),
            },
        },
        Check::NameCollision { collision: false },
    ]
    .into_iter()
    .enumerate()
    {
        inputs.push(review(Input::Knowledge {
            ordinal,
            input: Box::new(input),
        }));
    }
    inputs.push(review(Input::State {
        total: 1,
        terminal: 1,
        accepted: 1,
        editor_kind: "ai".into(),
        action: "accept_unchanged".into(),
    }));
    let outputs = inputs
        .into_iter()
        .enumerate()
        .map(|(n, input)| output(input, wrong && n == 12))
        .collect::<Vec<_>>();
    let directory = f.package.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    for id in ["literature.analysis-workflow", "literature.concepts"] {
        value["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"version":1}));
    }
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|ext| ext == "sql") {
            fs::copy(entry.path(), directory.join("data").join(entry.file_name())).unwrap();
        }
    }
    f.configure(&outputs, Some(12), false);
    (f, request)
}
#[tokio::test]
async fn suggestion_item_late_wait_releases_guard_rolls_back_written_term_and_rechecks_candidate() {
    let (f, request) = fixture(false);
    let before = counts(&f);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let call = tokio::spawn(async move { owner.decide_suggestion_item(store, key, request).await });
    assert!(f.entered(12).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(
            db.query_row("select count(*) from terms", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute(
            "update ai_candidates set review_state='rejected' where id='legacy-candidate'",
            [],
        )
        .unwrap();
    });
    let available = tokio::time::timeout(super::super::super::super::WAIT, &mut writer).await;
    f.release();
    let error = call.await.unwrap().unwrap_err();
    if available.is_err() {
        writer.await.unwrap();
    }
    assert!(available.is_ok());
    assert!(error.to_string().contains("plugin_host_source_changed"));
    assert_eq!(counts(&f), before);
    close(f).await;
}
#[tokio::test]
async fn suggestion_item_wrong_late_result_has_request_identity_and_leaves_no_prefix() {
    let (f, request) = fixture(true);
    let before = counts(&f);
    f.release();
    let error = decide(&f, request).await.unwrap_err();
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error else {
        panic!("native failure")
    };
    assert_eq!(failure.code, "plugin_host_response_mismatch");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    assert_eq!(counts(&f), before);
    assert!(f.store.connection().unwrap().is_autocommit());
    close(f).await;
}
#[tokio::test]
async fn suggestion_item_caller_exit_retains_owner_and_shutdown_settles_without_partial_writes() {
    for shutdown in [false, true] {
        let (f, request) = fixture(false);
        let before = counts(&f);
        let owner = f.service.clone();
        let store = f.store.clone();
        let key = f.key.clone();
        let call =
            tokio::spawn(async move { owner.decide_suggestion_item(store, key, request).await });
        assert!(f.entered(12).await);
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        assert!(f.service.doi_operations.try_write().is_err());
        if shutdown {
            let stopped = f.service.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            let owner = f.service.clone();
            let stop = tokio::spawn(async move { owner.shutdown().await });
            tokio::time::timeout(super::super::super::super::WAIT, &mut stopped)
                .await
                .unwrap();
            f.release();
            stop.await.unwrap().unwrap();
            assert_eq!(counts(&f), before);
        } else {
            f.release();
            let drained = tokio::time::timeout(
                super::super::super::super::WAIT,
                f.service.doi_operations.write(),
            )
            .await
            .unwrap();
            drop(drained);
            let after = counts(&f);
            assert_eq!(after.1 - before.1, 1);
            assert_eq!(after.5 - before.5, 1);
            f.service.shutdown().await.unwrap();
        }
        assert!(f.store.connection().unwrap().is_autocommit());
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}
