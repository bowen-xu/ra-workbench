//! Manual definitions keep the original owned operation and never borrow SQLite across Native waits.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::wire::*,
    catalog::CatalogOutput,
    knowledge_promotion::{KnowledgeValidationInputV1 as Input, manual::DefinitionClaimDraft},
    paper_context_wire::encode_payload,
};
use std::fs;
fn fixture() -> (Fixture, DefinitionClaimDraft) {
    let mut f = Fixture::new();
    super::matrix_fixture::seed(&f);
    f.store.connection().unwrap().execute_batch("insert into concepts(id,label,explanation,created_at,updated_at) values('definition-concept','Definition concept','','now','now');").unwrap();
    let draft = DefinitionClaimDraft {
        id: None,
        concept_id: "definition-concept".into(),
        statement: " Definition Ω ".into(),
        scope_note: " Scope ".into(),
        anchor_id: Some("matrix-first-anchor".into()),
        expected_revision: None,
        change_reason: None,
    };
    let inputs = [
        Input::DefinitionRevision {
            expected_revision: None,
            current_revision: None,
        },
        Input::ManualDefinition {
            concept_identity_sha256: crate::runtime_plugins::files::digest(
                draft.concept_id.as_bytes(),
            ),
            statement_is_empty: draft.statement.trim().is_empty(),
        },
    ];
    let outputs = inputs
        .into_iter()
        .enumerate()
        .map(|(ordinal, input)| {
            let decision = WorkflowDecisionV1::KnowledgeValidation {
                decision: super::wait_policy_fixtures::knowledge("manual_definition_waits", &input),
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
            .push(json!({"id":id,"version":1}));
    }
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    super::wait_policy_fixtures::install_knowledge_sql(&directory);
    f.configure(&outputs, Some(1), false);
    f.bind();
    (f, draft)
}
fn rows(f: &Fixture) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row("select count(*) from definition_claims", [], |r| r.get(0))
        .unwrap()
}
fn spawn(
    f: &Fixture,
    draft: DefinitionClaimDraft,
) -> tokio::task::JoinHandle<
    Result<
        research_workbench_native_sdk::knowledge_promotion::models::DefinitionClaim,
        crate::ai::error::AiError,
    >,
> {
    let service = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    tokio::spawn(async move { service.save_definition(store, key, draft).await })
}
#[tokio::test]
async fn manual_definition_wait_releases_guard_and_rejects_changed_canonical() {
    let (f, draft) = fixture();
    let call = spawn(&f, draft);
    if !f.entered(1).await {
        f.release();
        panic!("definition gate not reached: {:?}", call.await);
    }
    assert_eq!(rows(&f), 0);
    f.store.connection().unwrap().execute_batch("insert into concepts(id,label,explanation,created_at,updated_at) values('new-canonical','New canonical','','now','now');
    insert into concept_identity_events(id,operation,survivor_concept_id,source_concept_ids_json,snapshot_json,created_at) values('merge','merge','new-canonical','[\"definition-concept\"]','{}','now');
    insert into concept_identity_mappings(source_concept_id,canonical_concept_id,merge_event_id,created_at) values('definition-concept','new-canonical','merge','now');").unwrap();
    f.release();
    let error = call.await.unwrap().unwrap_err();
    assert_eq!(error.code(), "plugin_host_source_changed");
    assert_eq!(rows(&f), 0);
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
async fn manual_definition_aborted_caller_keeps_owner_and_shutdown_drains() {
    use std::{future::Future, task::Poll};
    for shutdown in [false, true] {
        let (f, draft) = fixture();
        let call = spawn(&f, draft);
        if !f.entered(1).await {
            f.release();
            panic!("definition gate not reached: {:?}", call.await);
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
                while rows(&f) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            f.service.shutdown().await.unwrap();
            assert_eq!(rows(&f), 1);
        }
    }
}

#[tokio::test]
async fn manual_definition_text_exceeding_native_frame_uses_only_finite_observations() {
    use std::{future::Future, task::Poll};
    let (f, mut draft) = fixture();
    draft.statement =
        "x".repeat(research_workbench_native_sdk::literature_trace_protocol::MAX_FRAME_BYTES + 1);
    let call = spawn(&f, draft);
    if !f.entered(1).await {
        f.release();
        panic!(
            "large original draft did not reach its finite validation gate: {:?}",
            call.await
        );
    }
    assert_eq!(rows(&f), 0);
    call.abort();
    let _ = call.await;
    let mut stopping = Box::pin(f.service.shutdown());
    assert!(std::future::poll_fn(|cx| Poll::Ready(stopping.as_mut().poll(cx).is_pending())).await);
    f.release();
    stopping.await.unwrap();
    assert_eq!(rows(&f), 0);
}
