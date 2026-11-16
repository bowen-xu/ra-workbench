//! Native waits use owned snapshots and release the original capture transaction.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::wire::*,
    catalog::CatalogOutput,
    concepts::NewConceptDetails,
    knowledge_promotion::{KnowledgeValidationInputV1 as Input, capture::*},
    paper_context_wire::encode_payload,
};
use std::fs;
fn fixture(wrong_family: bool) -> (Fixture, ConceptCaptureDraft) {
    let mut f = Fixture::new();
    super::matrix_fixture::seed(&f);
    let concept = NewConceptDetails {
        label: "Captured concept".into(),
        explanation: "Source definition".into(),
        disambiguation: "".into(),
        domain: "".into(),
        tradition: "".into(),
        period: "".into(),
        change_reason: None,
    };
    let draft = ConceptCaptureDraft {
        term_text: "Captured term".into(),
        language: None,
        anchor_id: "matrix-first-anchor".into(),
        usage_text: "".into(),
        resolution: ConceptResolutionDraft::CreateNew {
            concept: concept.clone(),
            definition_statement: None,
        },
    };
    let inputs = [
        Input::Capture {
            draft: Box::new(draft.clone()),
            quote: "Synthetic first result".into(),
        },
        Input::Term {
            text: draft.term_text.clone(),
            language: "".into(),
        },
        Input::Details { details: concept },
        Input::NameCollision { collision: false },
    ];
    let outputs = inputs
        .into_iter()
        .enumerate()
        .map(|(ordinal, input)| {
            let decision = if wrong_family && ordinal == 2 {
                WorkflowDecisionV1::SelectionEvidence { result: Ok(vec![]) }
            } else {
                WorkflowDecisionV1::KnowledgeValidation {
                    decision: super::wait_policy_fixtures::knowledge(
                        "concept_capture_waits",
                        &input,
                    ),
                }
            };
            let input = WorkflowInputV1::KnowledgeValidation {
                ordinal,
                input: Box::new(input),
            };
            CatalogOutput::AnalysisWorkflow {
                outcome: Box::new(WorkflowWireOutcomeV1 {
                    input: WorkflowWireInputV1 {
                        cbor_base64: encode_payload(&input).unwrap(),
                    },
                    decision_cbor_base64: encode_payload(&decision).unwrap(),
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
            .push(json!({"id": id, "version": 1}));
    }
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    super::wait_policy_fixtures::install_knowledge_sql(&directory);
    f.configure(&outputs, Some(2), false);
    f.bind();
    (f, draft)
}
fn spawn(
    f: &Fixture,
    draft: ConceptCaptureDraft,
) -> tokio::task::JoinHandle<Result<ConceptCaptureResult, crate::ai::error::AiError>> {
    let service = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    tokio::spawn(async move { service.capture_concept(store, key, draft).await })
}
fn rows(f: &Fixture) -> i64 {
    f.store.connection().unwrap().query_row("select (select count(*) from terms)+(select count(*) from concepts)+(select count(*) from term_usages)+(select count(*) from definition_claims)+(select count(*) from research_object_revisions)",[],|r|r.get(0)).unwrap()
}
#[tokio::test]
async fn human_capture_wait_rolls_back_prefix_and_rejects_changed_anchor_or_wrong_family() {
    for wrong in [false, true] {
        let (f, draft) = fixture(wrong);
        let call = spawn(&f, draft);
        let entered = f.entered(2).await;
        if !entered {
            f.release();
            let result = call.await;
            panic!("capture gate not reached: {result:?}");
        }
        assert_eq!(rows(&f), 0);
        if !wrong {
            f.store.connection().unwrap().execute("update source_anchors set quote_exact='changed' where id='matrix-first-anchor'",[]).unwrap();
        }
        f.release();
        let error = call.await.unwrap().unwrap_err();
        let value = serde_json::to_value(crate::ai::error::AiCommandError::from(error)).unwrap();
        assert_eq!(
            value["nativeFailure"]["code"],
            if wrong {
                "plugin_host_response_mismatch"
            } else {
                "plugin_host_source_changed"
            }
        );
        assert!(value["nativeFailure"]["requestId"].is_string());
        assert_eq!(rows(&f), 0);
        f.service.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn human_capture_aborted_caller_retains_owned_completion_and_shutdown_drains() {
    use std::{future::Future, task::Poll};
    for shutdown in [false, true] {
        let (f, draft) = fixture(false);
        let call = spawn(&f, draft);
        if !f.entered(2).await {
            f.release();
            let result = call.await;
            panic!("capture gate not reached: {result:?}");
        }
        assert_eq!(rows(&f), 0);
        call.abort();
        let _ = call.await;
        if shutdown {
            let mut stopping = Box::pin(f.service.shutdown());
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(stopping.as_mut().poll(cx).is_pending()))
                    .await
            );
            f.release();
            stopping.await.unwrap();
            assert_eq!(rows(&f), 0);
        } else {
            f.release();
            tokio::time::timeout(WAIT, async {
                loop {
                    if rows(&f) > 0 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            f.service.shutdown().await.unwrap();
            assert_eq!(rows(&f), 6);
        }
    }
}
