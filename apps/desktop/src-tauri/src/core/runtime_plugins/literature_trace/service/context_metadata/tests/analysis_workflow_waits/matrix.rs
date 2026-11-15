//! Matrix waits retain Core guards and bind every response to the selected sources.
use super::*;
#[tokio::test]
async fn matrix_outcomes_reject_unselected_sources_and_wrong_decision_family() {
    use research_workbench_native_sdk::{
        analysis_workflow::matrix::*,
        candidate_review::wire::{ReviewDecisionV1, ReviewInputV1},
        derived_view::DerivedValidationInputV1,
        generation::{AiMatrixSynthesisContext, AiMatrixSynthesisContextDraft},
    };
    let draft = AiMatrixSynthesisContextDraft {
        project_id: "project".into(),
        matrix_id: "matrix".into(),
        matrix_revision: 1,
        view_kind: "taxonomy".into(),
        title: "Scope".into(),
        scope_note: "Selected sources".into(),
        inputs: vec![],
    };
    let cases = [
        (
            WorkflowInputV1::MatrixContext {
                input: Box::new(MatrixContextInputV1::Finish {
                    draft: draft.clone(),
                    cells: vec![],
                    scope: vec![("anchor".into(), "selected".into())],
                }),
            },
            WorkflowDecisionV1::MatrixContext {
                decision: MatrixContextDecisionV1::Resolved {
                    result: Ok(AiMatrixSynthesisContext {
                        project_id: draft.project_id,
                        matrix_id: draft.matrix_id,
                        matrix_revision: 1,
                        view_kind: draft.view_kind,
                        title: draft.title,
                        scope_note: draft.scope_note,
                        inputs: vec![],
                        evidence_anchor_ids: vec!["unselected".into()],
                    }),
                },
            },
        ),
        (
            WorkflowInputV1::MatrixEvidence {
                input: Box::new(MatrixEvidenceInputV1::Sources { sources: vec![] }),
            },
            WorkflowDecisionV1::MatrixEvidence {
                decision: MatrixEvidenceDecisionV1::Validated { rejection: None },
            },
        ),
        (
            WorkflowInputV1::CandidateReview {
                input: Box::new(ReviewInputV1::DerivedValidation {
                    input: Box::new(DerivedValidationInputV1::Provenance {
                        provenance_kind: "ai_suggestion".into(),
                    }),
                }),
            },
            WorkflowDecisionV1::CandidateReview {
                decision: ReviewDecisionV1::Validated { rejection: None },
            },
        ),
    ];
    for (input, decision) in cases {
        let mut f = Fixture::new();
        let output = CatalogOutput::AnalysisWorkflow {
            outcome: Box::new(WorkflowWireOutcomeV1 {
                input: WorkflowWireInputV1 {
                    cbor_base64: encode_payload(&input).unwrap(),
                },
                decision_cbor_base64: encode_payload(&decision).unwrap(),
            }),
        };
        configure(&mut f, output);
        let error = f
            .service
            .analysis_workflow_decision(&f.key, input)
            .await
            .unwrap_err();
        assert_eq!(error.code, "plugin_host_response_mismatch");
        assert!(error.instance_id.is_some());
        assert!(error.request_id.is_some());
        let count:i64=f.store.connection().unwrap().query_row("select (select count(*) from ai_evidence_bundles)+(select count(*) from derived_research_views)",[],|row|row.get(0)).unwrap();
        assert_eq!(count, 0);
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn matrix_preview_wait_releases_guard_and_rechecks_original_matrix_revision() {
    use research_workbench_native_sdk::{
        analysis_workflow::matrix::*,
        generation::{AiMatrixSynthesisContextDraft, AiScopeItemDraft},
    };
    let mut f = Fixture::new();
    let (credentials, input) = actual_package::analysis_overview::seeded_task(&f);
    let mut draft =
        research_literature_library_native::analysis_workflow::overview_task::task(&input);
    draft.paper_template_ref = None;
    draft.overview_profile_selection_revision = None;
    super::super::matrix_fixture::seed(&f);
    f.store.connection().unwrap().execute_batch("insert into evidence_matrices(id,project_id,title,paper_ids_json,dimensions_json,current_revision,created_at,updated_at) values('waiting-matrix','matrix-project','Synthetic','[\"paper\",\"matrix-second\"]','[{\"id\":\"claim\",\"label\":\"Claims\"}]',1,'now','now');").unwrap();
    draft.task_kind = "matrix_synthesis".into();
    draft.suggestion_candidate_kind = None;
    draft.matrix_synthesis_context = Some(AiMatrixSynthesisContextDraft {
        project_id: "matrix-project".into(),
        matrix_id: "waiting-matrix".into(),
        matrix_revision: 1,
        view_kind: "taxonomy".into(),
        title: "Synthetic".into(),
        scope_note: "Selected sources".into(),
        inputs: vec![],
    });
    draft.scope_items = ["matrix-first-anchor", "matrix-second-anchor"]
        .into_iter()
        .map(|id| AiScopeItemDraft {
            item_kind: "anchor".into(),
            object_id: id.into(),
            parent_object_id: None,
            content_sha256: None,
            object_revision: None,
        })
        .collect();
    let header = MatrixContextInputV1::Header {
        draft: draft.matrix_synthesis_context.clone().unwrap(),
    };
    let revision = MatrixContextInputV1::Revision { matches: true };
    let inputs = [
        (
            WorkflowInputV1::TaskShape {
                draft_cbor_base64: encode_payload(&draft).unwrap(),
            },
            WorkflowDecisionV1::Validated { rejection: None },
        ),
        (
            WorkflowInputV1::TaskScope {
                task_kind: "matrix_synthesis".into(),
                item_kinds: vec!["anchor".into(), "anchor".into()],
            },
            WorkflowDecisionV1::Validated { rejection: None },
        ),
        (
            WorkflowInputV1::MatrixContext {
                input: Box::new(header.clone()),
            },
            WorkflowDecisionV1::MatrixContext {
                decision: research_literature_library_native::analysis_workflow::matrix::context(
                    header,
                ),
            },
        ),
        (
            WorkflowInputV1::MatrixContext {
                input: Box::new(revision.clone()),
            },
            WorkflowDecisionV1::MatrixContext {
                decision: research_literature_library_native::analysis_workflow::matrix::context(
                    revision,
                ),
            },
        ),
    ];
    let outputs = inputs
        .into_iter()
        .map(|(input, decision)| CatalogOutput::AnalysisWorkflow {
            outcome: Box::new(WorkflowWireOutcomeV1 {
                input: WorkflowWireInputV1 {
                    cbor_base64: encode_payload(&input).unwrap(),
                },
                decision_cbor_base64: encode_payload(&decision).unwrap(),
            }),
        })
        .collect::<Vec<_>>();
    let directory = f.package.path().join("writing/data");
    for (name, sql) in [
        "matrix-synthesis-matches.v1.sql",
        "matrix-synthesis-cell.v1.sql",
    ]
    .into_iter()
    .zip(research_literature_library_native::analysis_workflow::matrix::static_sql())
    {
        fs::write(directory.join(name), sql).unwrap();
    }
    configure(&mut f, outputs[0].clone());
    f.configure(&outputs, Some(3), false);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let call = tokio::spawn(async move {
        owner
            .preview_analysis_overview(store, credentials, &key, draft)
            .await
    });
    assert!(f.entered(3).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update evidence_matrices set current_revision=2 where id='waiting-matrix'",
            [],
        )
        .unwrap();
    });
    let available = tokio::time::timeout(WAIT, &mut writer).await;
    f.release();
    let error = tokio::time::timeout(WAIT, call)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(available.is_ok());
    assert!(error.to_string().contains("plugin_host_source_changed"));
    let count: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("select count(*) from ai_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    f.service.shutdown().await.unwrap();
}
