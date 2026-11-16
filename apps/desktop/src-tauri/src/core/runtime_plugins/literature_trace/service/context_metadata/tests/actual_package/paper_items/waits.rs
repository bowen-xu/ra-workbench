//! Late paper-concept decisions retain original guard-free reentry and owned completion.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::{candidate::CandidateInputV1, wire::*},
    candidate_review::{
        items::{ItemReviewDecisionV1, ItemReviewInputV1},
        paper::*,
    },
    catalog::CatalogOutput,
    concepts::NewConceptDetails,
    knowledge_promotion::{KnowledgePromotionV1, KnowledgeValidationInputV1 as Check},
    paper_context_wire::encode_payload,
};
fn fixture(wrong: bool) -> (Fixture, AiSuggestionItemDecisionDraft) {
    let mut f = Fixture::new();
    let mut value = payload();
    value["concepts"].as_array_mut().unwrap().truncate(1);
    value["relations"] = json!([]);
    let mut candidate = generic_reading::seed(&f, value.clone());
    let decoded = protocol::decode_candidate_value(
        value.clone(),
        &std::collections::HashSet::from(["ev-quote".into()]),
    )
    .unwrap();
    for reference in decoded.citation_references() {
        candidate = crate::ai::review_support::record_citation_assessment_with(
            &f.store.connection().unwrap(),
            research_workbench_native_sdk::candidate_review::contracts::AiCitationAssessmentDraft {
                candidate_id: candidate.id.clone(),
                candidate_revision: 1,
                statement_id: reference.statement_id.clone(),
                evidence_item_id: "legacy-item".into(),
                expected_assessment_revision: 0,
                semantic_relation: "supports".into(),
                rationale: "Synthetic checked source".into(),
                idempotency_key: reference.statement_id,
            },
            |_| Ok::<_, crate::ai::error::AiError>(decoded.clone()),
        )
        .unwrap();
    }
    f.store.connection().unwrap().execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values('inspection','legacy-candidate',1,'legacy-item','now')",[]).unwrap();
    let draft = request(
        &candidate,
        "concept-1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let item = PaperItemV1::Concept(Box::new(
        serde_json::from_value(value["concepts"][0].clone()).unwrap(),
    ));
    let promotion = Input::Promotion {
        item: item.clone(),
        resolution: draft.resolution.clone(),
        paper_id: "paper".into(),
        candidate_id: candidate.id.clone(),
        candidate_revision: 1,
        sources: vec![("ev-quote".into(), "legacy-anchor".into())],
    };
    let Decision::Promotion { result: Ok(plan) } = protocol::paper_review(promotion.clone()) else {
        panic!("paper plan")
    };
    let KnowledgePromotionV1::PaperConcept(paper) = *plan else {
        panic!("paper")
    };
    let review = |input| WorkflowInputV1::PaperConceptReview {
        input: Box::new(input),
    };
    let mut inputs = vec![
        review(Input::Draft {
            draft: draft.clone(),
        }),
        review(Input::Candidate {
            revision: 1,
            current_revision: 1,
            review_state: candidate.review_state,
        }),
        WorkflowInputV1::Candidate {
            input: Box::new(CandidateInputV1::Decode {
                content: value.to_string(),
                evidence_keys: vec!["ev-quote".into()],
            }),
        },
        review(Input::Select {
            payload: value,
            suggestion_id: "concept-1".into(),
        }),
        review(Input::Action {
            draft: draft.clone(),
            editor_kind: "ai".into(),
            item: item.clone(),
        }),
        review(Input::Evidence { item }),
        review(Input::Inspection { inspected: true }),
        review(Input::Paper {
            paper_ids: vec!["paper".into()],
        }),
        review(promotion),
    ];
    for (ordinal, input) in [
        Check::PaperConcept {
            draft: Box::new(paper),
        },
        Check::Origin {
            provenance_kind: "ai_accepted_unchanged".into(),
            anchor_ids: vec!["legacy-anchor".into()],
        },
        Check::UniqueAnchor { duplicate: false },
        Check::Term {
            text: "Routing gate".into(),
            language: "en".into(),
        },
        Check::Details {
            details: NewConceptDetails {
                label: "Routing gate".into(),
                explanation: "Routing gate explanation".into(),
                disambiguation: String::new(),
                domain: String::new(),
                tradition: String::new(),
                period: String::new(),
                change_reason: Some("ai_suggestion_acceptance".into()),
            },
        },
        Check::NameCollision { collision: false },
        Check::Term {
            text: "Routing gate alias".into(),
            language: "en".into(),
        },
        Check::Origin {
            provenance_kind: "ai_accepted_unchanged".into(),
            anchor_ids: vec!["legacy-anchor".into()],
        },
        Check::UniqueAnchor { duplicate: false },
        Check::Definition {
            statement: "Routing gate is defined.".into(),
            scope_note: "paper scope".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        inputs.push(WorkflowInputV1::SuggestionReview {
            input: Box::new(ItemReviewInputV1::Knowledge {
                ordinal,
                input: Box::new(input),
            }),
        });
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
        .map(|(ordinal, input)| {
            let decision = match &input {
                WorkflowInputV1::Candidate { input } => WorkflowDecisionV1::Candidate {
                    decision: protocol::candidate(*input.clone()),
                },
                WorkflowInputV1::PaperConceptReview { input } => {
                    WorkflowDecisionV1::PaperConceptReview {
                        decision: protocol::paper_review(*input.clone()),
                    }
                }
                WorkflowInputV1::SuggestionReview { input } => {
                    WorkflowDecisionV1::SuggestionReview {
                        decision: protocol::item_review(*input.clone()),
                    }
                }
                _ => unreachable!(),
            };
            CatalogOutput::AnalysisWorkflow {
                outcome: Box::new(WorkflowWireOutcomeV1 {
                    input: WorkflowWireInputV1 {
                        cbor_base64: encode_payload(&input).unwrap(),
                    },
                    decision_cbor_base64: encode_payload(&if wrong && ordinal == 15 {
                        WorkflowDecisionV1::SuggestionReview {
                            decision: ItemReviewDecisionV1::State {
                                state: "rejected".into(),
                            },
                        }
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
    for id in ["literature.analysis-workflow", "literature.concepts"] {
        value["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"version":1}));
    }
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|ext| ext == "sql") {
            fs::copy(entry.path(), directory.join("data").join(entry.file_name())).unwrap();
        }
    }
    f.configure(&outputs, Some(15), false);
    (f, draft)
}
#[tokio::test]
async fn paper_item_alias_wait_rolls_back_original_concept_releases_guard_and_rechecks_candidate() {
    let (f, draft) = fixture(false);
    let before = counts(&f);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let call =
        tokio::spawn(async move { owner.decide_paper_concept_item(store, key, draft).await });
    assert!(f.entered(15).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(
            db.query_row("select count(*) from concepts", [], |r| r.get::<_, i64>(0))
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
    assert_eq!(counts(&f), before);
    close(f).await;
}
#[tokio::test]
async fn paper_item_wrong_alias_rule_reply_leaves_no_knowledge_or_role_prefix() {
    let (f, draft) = fixture(true);
    let before = counts(&f);
    f.release();
    let error = decide(&f, draft).await.unwrap_err();
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
async fn paper_item_caller_exit_retains_original_owner_and_shutdown_settles_all_rows() {
    for shutdown in [false, true] {
        let (f, draft) = fixture(false);
        let before = counts(&f);
        let owner = f.service.clone();
        let store = f.store.clone();
        let key = f.key.clone();
        let call =
            tokio::spawn(async move { owner.decide_paper_concept_item(store, key, draft).await });
        assert!(f.entered(15).await);
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        assert!(f.service.doi_operations.try_write().is_err());
        if shutdown {
            let stopped = f.service.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            let owner = f.service.clone();
            let stop = tokio::spawn(async move { owner.shutdown().await });
            tokio::time::timeout(super::super::super::WAIT, &mut stopped)
                .await
                .unwrap();
            f.release();
            stop.await.unwrap().unwrap();
            assert_eq!(counts(&f), before);
        } else {
            f.release();
            let drained =
                tokio::time::timeout(super::super::super::WAIT, f.service.doi_operations.write())
                    .await
                    .unwrap();
            drop(drained);
            assert_eq!(counts(&f), (2, 1, 1, 0, 1, 1));
            f.service.shutdown().await.unwrap();
        }
        assert!(f.store.connection().unwrap().is_autocommit());
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}
