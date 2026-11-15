//! Original matrix synthesis through real Core and the independently built package.
use super::super::matrix_fixture;
use super::*;
use crate::{
    ai::{
        CancellationSignal, JsonCredentialStore,
        generation_support::{AiRun, AiTaskSpecDraft, execute_research_task},
    },
    runtime_plugins::literature_matrix::{MatrixRequest, MatrixResult},
};
use research_workbench_native_sdk::{
    candidate_review::contracts::{
        AiCandidate, AiCandidateRevisionDraft, AiCitationAssessmentDraft, AiReviewDecisionDraft,
    },
    generation::{AiMatrixSynthesisContextDraft, AiScopeItemDraft, DerivedViewInputRef},
};
mod execution;
mod failures;

pub(super) async fn setup() -> (Fixture, JsonCredentialStore, AiTaskSpecDraft) {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut task) = analysis_overview::suggestion_task(&f, "concept_suggestion_v1");
    matrix_fixture::seed(&f);
    let selection = task.model_selection.as_mut().unwrap();
    selection.provider_id = "openai".into();
    selection.model_id = "gpt-5.6-terra".into();
    f.store.connection().unwrap().execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('openai',1,'https://api.openai.com',60000,'now') on conflict(provider_id) do update set enabled=1",[]).unwrap();
    crate::agent::skills::bundled::install_defaults(f.store.agent_configuration_directory())
        .unwrap();
    f.bind_actual_package(catalog, key);
    let MatrixResult::Matrix(matrix) = f
        .service
        .evidence_matrix(
            f.store.shared_agent_connection(),
            f.key.clone(),
            MatrixRequest::Save {
                draft: Box::new(matrix_fixture::draft()),
            },
        )
        .await
        .unwrap()
    else {
        panic!("matrix result");
    };
    let mut inputs = Vec::new();
    for (paper, anchor) in [
        ("paper", "matrix-first-anchor"),
        ("matrix-second", "matrix-second-anchor"),
    ] {
        let mut draft = matrix_fixture::cell(&matrix.id);
        draft.paper_id = paper.into();
        draft.coverage_state = "populated".into();
        let mut statement = matrix_fixture::statement();
        statement.evidence_anchor_ids = vec![anchor.into()];
        draft.statements = vec![statement];
        let MatrixResult::Cell(cell) = f
            .service
            .evidence_matrix(
                f.store.shared_agent_connection(),
                f.key.clone(),
                MatrixRequest::SaveCell {
                    draft: Box::new(draft),
                },
            )
            .await
            .unwrap()
        else {
            panic!("cell result");
        };
        inputs.push(DerivedViewInputRef {
            object_kind: "matrix_cell".into(),
            object_id: cell.id,
            object_revision: cell.revision,
            selection_reason: "Explicit synthetic input Ω".into(),
        });
    }
    task.task_kind = "matrix_synthesis".into();
    task.suggestion_candidate_kind = None;
    task.user_goal =
        "Research question. Do not invoke any tool. Compare only the frozen evidence.".into();
    task.pipeline_version = "matrix-synthesis-runtime-v1".into();
    task.prompt_version = "matrix-synthesis-grounded-v1".into();
    task.max_input_tokens = 16_000;
    task.max_output_tokens = 3_000;
    task.timeout_ms = 90_000;
    task.scope_items = ["matrix-first-anchor", "matrix-second-anchor"]
        .into_iter()
        .map(|id| AiScopeItemDraft {
            item_kind: "anchor".into(),
            object_id: id.into(),
            parent_object_id: None,
            content_sha256: None,
            object_revision: None,
        })
        .collect();
    task.matrix_synthesis_context = Some(AiMatrixSynthesisContextDraft {
        project_id: "matrix-project".into(),
        matrix_id: matrix.id,
        matrix_revision: matrix.revision,
        view_kind: "taxonomy".into(),
        title: "  Frozen comparison Ω  ".into(),
        scope_note: "  Two selected cells only.  ".into(),
        inputs,
    });
    (f, credentials, task)
}
fn stable(preview: &crate::ai::generation_support::AiTaskPreview) -> Value {
    let mut value = serde_json::to_value(preview).unwrap();
    for scope in value["scopeItems"].as_array_mut().unwrap() {
        scope.as_object_mut().unwrap().remove("id");
        scope.as_object_mut().unwrap().remove("createdAt");
    }
    value
}
async fn admit(
    f: &Fixture,
    credentials: &JsonCredentialStore,
    mut draft: AiTaskSpecDraft,
) -> AiRun {
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &f.key, draft.clone())
        .await
        .unwrap();
    draft.confirmed_scope_hash = Some(preview.scope_hash);
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    f.service
        .admit_analysis_overview(
            f.store.clone(),
            credentials.clone(),
            f.key.clone(),
            draft,
            None,
        )
        .await
        .unwrap()
}
fn payload() -> Value {
    json!({"kind":"derived_research_view_v1","schemaVersion":1,"viewKind":"taxonomy","title":"Frozen comparison Ω","scopeNote":"Two selected cells only.","items":[{"id":"comparison","text":"Two sources support this provisional comparison.","epistemicRole":"synthesis","evidenceItemIds":["ev-source-1","ev-source-2"],"hasConflict":true}]})
}
async fn generated() -> (Fixture, AiCandidate) {
    let (f, credentials, draft) = setup().await;
    let run = admit(&f, &credentials, draft).await;
    let signal = CancellationSignal::default();
    let (host, wire) =
        task_generation_host::Host::new(&f, credentials, run.clone(), signal.clone(), payload());
    execute_research_task(&host, &run, &signal).await.unwrap();
    assert_eq!(wire.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let candidate = {
        let db = f.store.connection().unwrap();
        let id: String = db
            .query_row(
                "select id from ai_candidates where run_id=?1",
                [&run.id],
                |row| row.get(0),
            )
            .unwrap();
        crate::ai::review_support::ReviewFacts::new(&db)
            .candidate(&id)
            .unwrap()
    };
    drop(host);
    (f, candidate)
}
async fn qualify(f: &Fixture, candidate: &AiCandidate) -> AiCandidate {
    let (value, _) = crate::ai::review_support::current_payload_source(candidate).unwrap();
    let payload: research_workbench_native_sdk::candidate_contracts::CandidatePayload =
        serde_json::from_value(value).unwrap();
    for reference in payload.citation_references() {
        for key in reference.evidence_item_ids {
            let item = candidate
                .evidence_bundle
                .items
                .iter()
                .find(|item| &item.citation_key == key)
                .unwrap();
            f.service
                .record_candidate_citation_assessment(
                    f.store.clone(),
                    f.key.clone(),
                    AiCitationAssessmentDraft {
                        candidate_id: candidate.id.clone(),
                        candidate_revision: candidate.current_revision,
                        statement_id: reference.statement_id.clone(),
                        evidence_item_id: item.id.clone(),
                        expected_assessment_revision: 0,
                        semantic_relation: "supports".into(),
                        rationale: "Checked synthetic evidence.".into(),
                        idempotency_key: format!("matrix-{}-{key}", candidate.current_revision),
                    },
                )
                .await
                .unwrap();
        }
    }
    f.store.connection().unwrap().execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values(?1,?2,?3,?4,'now')", rusqlite::params![uuid::Uuid::new_v4().to_string(),candidate.id,candidate.current_revision,candidate.evidence_bundle.items[0].id]).unwrap();
    crate::ai::review_support::ReviewFacts::new(&f.store.connection().unwrap())
        .candidate(&candidate.id)
        .unwrap()
}
fn review(candidate: &AiCandidate, action: &str) -> AiReviewDecisionDraft {
    AiReviewDecisionDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: candidate.current_revision,
        action: action.into(),
        rationale: "Checked selected matrix evidence Ω".into(),
        idempotency_key: format!("matrix-review-{action}"),
        reading_unit_title: None,
        project_id: Some("matrix-project".into()),
    }
}
async fn close(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_synthesis_preview_matches_original_and_preserves_error_order() {
    let (f, credentials, draft) = setup().await;
    let expected = prepared_preview(&f.store.connection().unwrap(),
        &credentials,
        &draft,
    )
    .unwrap();
    let actual = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &f.key, draft.clone())
        .await
        .unwrap();
    assert_eq!(stable(&actual), stable(&expected));
    assert_eq!(actual.evidence_preview.len(), 2);
    for case in 0..5 {
        let mut changed = draft.clone();
        let context = changed.matrix_synthesis_context.as_mut().unwrap();
        match case {
            0 => {
                context.title.clear();
                context.matrix_revision = 200;
            }
            1 => context.matrix_revision = 200,
            2 => {
                context.inputs[0].object_kind = "wrong".into();
                context.inputs[1].object_id = "absent".into();
            }
            3 => context.inputs[0].object_revision = 200,
            _ => changed.scope_items.reverse(),
        }
        let old = prepared_preview(&f.store.connection().unwrap(),
            &credentials,
            &changed,
        )
        .unwrap_err();
        let actual = f
            .service
            .preview_analysis_overview(f.store.clone(), credentials.clone(), &f.key, changed)
            .await
            .unwrap_err();
        assert_eq!(actual.code(), old.code());
        assert!(
            actual.to_string().contains(&old.to_string()),
            "{actual} / {old}"
        );
    }
    close(f).await;
}
