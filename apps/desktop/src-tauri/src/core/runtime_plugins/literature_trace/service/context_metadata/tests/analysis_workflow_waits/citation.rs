//! Citation decoding releases the original guard and retains accepted work ownership.
use super::*;
use crate::ai::{
    generation_support::{hash_text, save_prepared_candidate},
    review_support::{ReviewFacts, current_payload_source},
    task_evidence_support::{TaskEvidenceFacts, persist_selection_evidence},
};
use research_workbench_native_sdk::{
    analysis_workflow::candidate::CandidateInputV1,
    candidate_review::contracts::AiCitationAssessmentDraft,
};

pub(super) fn fixture(wrong: bool) -> (Fixture, AiCitationAssessmentDraft) {
    let (mut f, run, _) = pages::fixture("reference_extraction");
    let candidate = {
        let mut db = f.store.connection().unwrap();
        let facts = TaskEvidenceFacts::new(&db);
        let (scope, _) = facts.selection_scope(&run).unwrap();
        let source = facts.anchor("reference-anchor").unwrap().unwrap();
        let bundle =
            persist_selection_evidence(&mut db, &run, &scope, &source, &source.quote_exact, false)
                .unwrap();
        let content = json!({"kind":"reference_record_candidate_v1","schemaVersion":1,"rawText":"Synthetic reference.","evidenceItemIds":["ev-quote"],"fields":[{"name":"title","value":"Synthetic reference","evidenceItemIds":["ev-quote"]}],"uncertainties":[]}).to_string();
        let prepared = actual_package::prepared_candidate(&db, &run, &bundle, &content).unwrap();
        let saved = save_prepared_candidate(
            &mut db,
            &run,
            &bundle,
            prepared,
        )
        .unwrap();
        ReviewFacts::new(&db).candidate(&saved.id).unwrap()
    };
    let (value, keys) = current_payload_source(&candidate).unwrap();
    let input = CandidateInputV1::Decode {
        content: value.to_string(),
        evidence_keys: keys,
    };
    let decision =
        research_literature_library_native::analysis_workflow::candidate::decide(input.clone())
            .unwrap();
    let request = WorkflowInputV1::Candidate {
        input: Box::new(input),
    };
    let output = CatalogOutput::AnalysisWorkflow {
        outcome: Box::new(WorkflowWireOutcomeV1 {
            input: WorkflowWireInputV1 {
                cbor_base64: encode_payload(&request).unwrap(),
            },
            decision_cbor_base64: encode_payload(&if wrong {
                WorkflowDecisionV1::Validated { rejection: None }
            } else {
                WorkflowDecisionV1::Candidate { decision }
            })
            .unwrap(),
        }),
    };
    f.configure(&[output], Some(0), false);
    let draft = AiCitationAssessmentDraft {
        candidate_id: candidate.id,
        candidate_revision: 1,
        statement_id: "reference-field:title".into(),
        evidence_item_id: candidate.evidence_bundle.items[0].id.clone(),
        expected_assessment_revision: 0,
        semantic_relation: "supports".into(),
        rationale: String::new(),
        idempotency_key: "citation-controlled-wait".into(),
    };
    (f, draft)
}
fn count(f: &Fixture) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row("select count(*) from ai_citation_assessments", [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[tokio::test]
async fn citation_wait_releases_guard_and_rechecks_original_candidate_revision() {
    let (f, draft) = fixture(false);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let candidate = draft.candidate_id.clone();
    let call = tokio::spawn(async move {
        owner
            .record_candidate_citation_assessment(store, key, draft)
            .await
    });
    assert!(f.entered(0).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        let content: String = db
            .query_row(
                "select payload_json from ai_candidate_revisions where candidate_id=?1",
                [&candidate],
                |row| row.get(0),
            )
            .unwrap();
        db.execute("insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,parent_revision,change_reason,created_at) values('concurrent-revision',?1,2,?2,?3,'human',null,1,'controlled wait','now')", rusqlite::params![candidate,content,hash_text(&content)]).unwrap();
        db.execute(
            "update ai_candidates set current_revision=2 where id=?1",
            [&candidate],
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
    assert!(released.is_ok(), "Native wait must release the Core guard");
    assert!(error.to_string().contains("候选版本已变化"));
    assert_eq!(count(&f), 0);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn citation_wrong_native_decode_writes_nothing_and_preserves_request_identity() {
    let (f, draft) = fixture(true);
    f.release();
    let error = f
        .service
        .record_candidate_citation_assessment(f.store.clone(), f.key.clone(), draft)
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
async fn accepted_citation_survives_caller_loss_and_drains_on_shutdown() {
    for close in [false, true] {
        let (f, draft) = fixture(false);
        let owner = f.service.clone();
        let store = f.store.clone();
        let key = f.key.clone();
        let call = tokio::spawn(async move {
            owner
                .record_candidate_citation_assessment(store, key, draft)
                .await
        });
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
        assert!(f.store.connection().unwrap().is_autocommit());
        assert_eq!(count(&f), if close { 0 } else { 1 });
    }
}
