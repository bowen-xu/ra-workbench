//! Persisted unmapped historical candidates, reviewed by real Core and a new independent package.
use super::*;
use crate::ai::{error::AiError, review_support::ReviewFacts};
use research_workbench_native_sdk::candidate_review::contracts::{
    AiCandidate, AiCandidateRevisionDraft, AiCitationAssessmentDraft, AiReviewDecisionDraft,
};
use rusqlite::params;
mod failures;
mod waits;

fn payload(overview: bool) -> Value {
    let statement = |id: &str, role: &str| json!({"id":id,"text":format!("Synthetic historical {id} Ω"),"epistemicRole":role,"evidenceItemIds":["ev-quote"]});
    if overview {
        json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"reviewed_profile_composition_v1","profileRefs":[{"profileId":"legacy","profileVersion":1}],"fields":[
            {"field":"approach","status":"filled","statements":[statement("s1","source_paraphrase")]},
            {"field":"conclusion","status":"filled","statements":[statement("s2","inference")]}
        ]})
    } else {
        json!({"kind":"selection_explanation_v1","schemaVersion":1,"statements":[statement("s1","source_paraphrase")],"uncertainties":["Historical scope only."]})
    }
}
pub(super) fn seed(f: &Fixture, value: Value) -> AiCandidate {
    // Model a genuine pre-shared-Agent history row. No current binding is removed or forged.
    let _ = analysis_overview::seeded_task(f);
    let db = f.store.connection().unwrap();
    let bytes = value.to_string();
    let hash = crate::ai::generation_support::hash_text(&bytes);
    let kind = value["kind"].as_str().unwrap();
    let task = if kind == "selection_explanation_v1" {
        "selection_explanation"
    } else if kind == "paper_concept_map_v1" {
        "paper_concept_map"
    } else {
        "paper_overview"
    };
    let provider: String = db
        .query_row(
            "select provider_id from ai_provider_settings order by provider_id limit 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,created_at) values('legacy-anchor','paper','selected',?1,0,'1','[]','Synthetic evidence Ω','','','old')",["a".repeat(64)]).unwrap();
    db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values('legacy-run',?1,'needs_review',?2,'historical','v1','v1',1,?3,1000,1000,1000,'old','old')",params![task,provider,hash]).unwrap();
    if let Some(refs) = value.get("profileRefs") {
        // Historical composed overviews retain the original run-frozen profile frame.
        let fields: Vec<_> = value["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["field"].as_str().unwrap())
            .collect();
        db.execute("insert into ai_overview_profile_runs(run_id,paper_id,selection_revision,profile_refs_json,field_kinds_json,created_at) values('legacy-run','paper',1,?1,?2,'old')", params![refs.to_string(),serde_json::to_string(&fields).unwrap()]).unwrap();
    }
    db.execute("insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at) values('legacy-bundle','legacy-run',?1,'v1','valid','old')",[&hash]).unwrap();
    db.execute("insert into ai_evidence_items(id,bundle_id,citation_key,ordinal,anchor_id,paper_id,paper_title,attachment_id,content_sha256,page_index,page_label,quote_exact,quote_prefix,quote_suffix,context_text,relation_kind,validation_status,created_at) values('legacy-item','legacy-bundle','ev-quote',0,'legacy-anchor','paper','Paper','selected',?1,0,'1','Synthetic evidence Ω','','','Synthetic evidence Ω','direct_quote','valid','old')",["a".repeat(64)]).unwrap();
    db.execute("insert into ai_candidates(id,run_id,evidence_bundle_id,candidate_kind,schema_version,validation_status,original_payload_hash,created_at,updated_at) values('legacy-candidate','legacy-run','legacy-bundle',?1,1,'valid',?2,'old','old')",params![kind,hash]).unwrap();
    db.execute("insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,parent_revision,change_reason,created_at) values('legacy-revision','legacy-candidate',1,?1,?2,'ai','legacy-run',null,'','old')",params![bytes,hash]).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_legacy_ai_run_bindings where ai_run_id='legacy-run'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    ReviewFacts::new(&db).candidate("legacy-candidate").unwrap()
}
async fn setup(value: Value) -> (Fixture, AiCandidate) {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let candidate = seed(&f, value);
    f.bind_actual_package(catalog, key);
    (f, candidate)
}
pub(super) async fn qualify(f: &Fixture, candidate: &AiCandidate) -> AiCandidate {
    let (value, _) = crate::ai::review_support::current_payload_source(candidate).unwrap();
    // Decode by the existing candidate decoder, which is deliberately broader than untagged Serde.
    let decoded =
        research_literature_library_native::analysis_workflow::candidate::decode_candidate_value(
            value,
            &std::collections::HashSet::from(["ev-quote".into()]),
        );
    let decoded = decoded.unwrap();
    let references = decoded.citation_references();
    for reference in references {
        f.service
            .record_candidate_citation_assessment(
                f.store.clone(),
                f.key.clone(),
                AiCitationAssessmentDraft {
                    candidate_id: candidate.id.clone(),
                    candidate_revision: candidate.current_revision,
                    statement_id: reference.statement_id.clone(),
                    evidence_item_id: "legacy-item".into(),
                    expected_assessment_revision: 0,
                    semantic_relation: "supports".into(),
                    rationale: "Synthetic original source checked.".into(),
                    idempotency_key: format!("qualify-{}", uuid::Uuid::new_v4()),
                },
            )
            .await
            .unwrap();
    }
    f.store.connection().unwrap().execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values(?1,?2,?3,'legacy-item','now') on conflict(candidate_id,candidate_revision,evidence_item_id) do nothing",params![uuid::Uuid::new_v4().to_string(),candidate.id,candidate.current_revision]).unwrap();
    ReviewFacts::new(&f.store.connection().unwrap())
        .candidate(&candidate.id)
        .unwrap()
}
fn draft(candidate: &AiCandidate, action: &str) -> AiReviewDecisionDraft {
    AiReviewDecisionDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: candidate.current_revision,
        action: action.into(),
        rationale: "Historical source checked.".into(),
        idempotency_key: format!("review-{action}"),
        reading_unit_title: Some("Human Ω".into()),
        project_id: None,
    }
}
async fn decide(f: &Fixture, request: AiReviewDecisionDraft) -> Result<AiCandidate, AiError> {
    f.service
        .decide_analysis_candidate(f.store.clone(), f.key.clone(), request)
        .await
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from structured_reading_units),(select count(*) from reading_unit_evidence_links),(select count(*) from research_object_revisions where object_kind='reading_unit'),(select count(*) from ai_review_decisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}
async fn close(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_generic_reading_accept_edit_replay_and_reopen() {
    for overview in [false, true] {
        for edited in [false, true] {
            let (mut f, mut candidate) = setup(payload(overview)).await;
            if edited {
                let (mut value, _) =
                    crate::ai::review_support::current_payload_source(&candidate).unwrap();
                if overview {
                    value["fields"][0]["statements"][0]["text"] = json!("Human correction Ω");
                } else {
                    value["statements"][0]["text"] = json!("Human correction Ω");
                }
                candidate = f
                    .service
                    .revise_analysis_candidate(
                        f.store.clone(),
                        f.key.clone(),
                        AiCandidateRevisionDraft {
                            candidate_id: candidate.id.clone(),
                            expected_revision: 1,
                            payload: value,
                            change_reason: "Human correction".into(),
                        },
                    )
                    .await
                    .unwrap();
            }
            candidate = qualify(&f, &candidate).await;
            let request = draft(
                &candidate,
                if edited {
                    "accept_after_edit"
                } else {
                    "accept_unchanged"
                },
            );
            let accepted = decide(&f, request.clone()).await.unwrap();
            let units = if overview { 2 } else { 1 };
            assert_eq!(counts(&f), (units, units, units, 1));
            assert_eq!(
                accepted.decisions[0].target_object_kind.as_deref(),
                Some("reading_unit")
            );
            let expected = serde_json::to_value(&accepted).unwrap();
            assert_eq!(
                serde_json::to_value(decide(&f, request.clone()).await.unwrap()).unwrap(),
                expected
            );
            f = reopen_actual_fixture(f, false).await;
            assert_eq!(
                serde_json::to_value(decide(&f, request).await.unwrap()).unwrap(),
                expected
            );
            assert_eq!(counts(&f), (units, units, units, 1));
            let rows: Vec<(String, String, String, String)> = {
                let db = f.store.connection().unwrap();
                let mut s=db.prepare("select unit_kind,title,body,provenance_kind from structured_reading_units order by unit_kind").unwrap();
                s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap()
            };
            assert!(rows.iter().all(|r| r.1.starts_with("Human Ω")
                && r.3
                    == if edited {
                        "ai_accepted_after_edit"
                    } else {
                        "ai_accepted_unchanged"
                    }));
            assert!(rows.iter().any(|r| r.2.contains(if edited {
                "Human correction Ω"
            } else {
                "Synthetic historical"
            })));
            close(f).await;
        }
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_generic_reading_reject_defer_do_not_promote() {
    for overview in [false, true] {
        for action in ["reject", "defer"] {
            let (f, candidate) = setup(payload(overview)).await;
            let result = decide(&f, draft(&candidate, action)).await.unwrap();
            assert_eq!(
                result.review_state,
                if action == "reject" {
                    "rejected"
                } else {
                    "deferred"
                }
            );
            assert_eq!(counts(&f), (0, 0, 0, 1));
            close(f).await;
        }
    }
}
