//! Original paper-map acceptance from synthetic historical candidates with a real package.
use super::*;
mod failures;
mod waits;
use crate::ai::review_support::{AiCandidate, ReviewFacts};
use research_workbench_native_sdk::{
    analysis_workflow::wire::*,
    candidate_review::{
        items::{AiSuggestionItemDecisionDraft, AiSuggestionResolutionDraft},
        paper::{PaperReviewDecisionV1 as Decision, PaperReviewInputV1 as Input},
    },
};
fn payload() -> Value {
    let concept = |id: &str, term: &str, function: &str, novelty: &str| {
        json!({
            "id":id,"preferredTerm":term,
            "surfaceForms":[{"text":term,"language":"en","relationKind":"preferred","evidenceItemIds":["ev-quote"]},{"text":format!("{term} alias"),"language":"en","relationKind":"alias","evidenceItemIds":[]}],
            "paperFunction":function,"noveltyStatus":novelty,"explanationStatus":"explained_in_paper","explanation":format!("{term} explanation"),"disambiguation":"","roleRationale":"The paper uses this as a key concept.","noveltyRationale":"Grounded in the source.",
            "definitionClaims":[{"id":format!("definition-{id}"),"statement":format!("{term} is defined."),"scopeNote":"paper scope","evidenceItemIds":["ev-quote"]}],"evidenceItemIds":["ev-quote"],"referenceMentions":[]
        })
    };
    json!({"kind":"paper_concept_map_v1","schemaVersion":1,"concepts":[concept("concept-1","Routing gate","focal","introduced_by_paper"),concept("concept-2","Experts","foundational","pre_existing")],"relations":[{"id":"relation-1","subjectConceptCandidateId":"concept-1","predicateKind":"builds_on","objectConceptCandidateId":"concept-2","scopeNote":"within this paper","evidenceItemIds":["ev-quote"]}],"omissions":[]})
}
async fn setup(value: Value) -> (Fixture, AiCandidate) {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let candidate = generic_reading::seed(&f, value);
    f.bind_actual_package(catalog, key);
    let candidate = generic_reading::qualify(&f, &candidate).await;
    (f, candidate)
}
fn request(
    candidate: &AiCandidate,
    id: &str,
    action: &str,
    resolution: Option<AiSuggestionResolutionDraft>,
) -> AiSuggestionItemDecisionDraft {
    AiSuggestionItemDecisionDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: candidate.current_revision,
        suggestion_id: id.into(),
        action: action.into(),
        rationale: "Original synthetic source reviewed Ω".into(),
        idempotency_key: format!("{}-{id}-{action}", candidate.id),
        resolution,
    }
}
async fn decide(
    f: &Fixture,
    draft: AiSuggestionItemDecisionDraft,
) -> Result<AiCandidate, crate::ai::error::AiError> {
    f.service
        .decide_paper_concept_item(f.store.clone(), f.key.clone(), draft)
        .await
}
fn target(candidate: &AiCandidate, id: &str) -> String {
    candidate
        .suggestion_item_decisions
        .iter()
        .find(|d| d.suggestion_id == id && d.target_object_kind.as_deref() == Some("concept"))
        .and_then(|d| d.target_object_id.clone())
        .unwrap()
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from terms),(select count(*) from concepts),(select count(*) from definition_claims),(select count(*) from relation_assertions),(select count(*) from paper_concept_assertions),(select count(*) from ai_suggestion_item_decisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap()
}
async fn close(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_item_finite_draft_keeps_original_hash_and_rejection() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key.clone());
    let draft = AiSuggestionItemDecisionDraft {
        candidate_id: "historical".into(),
        candidate_revision: 1,
        suggestion_id: "concept-1".into(),
        action: "defer".into(),
        rationale: "Ω".into(),
        idempotency_key: "paper-decision".into(),
        resolution: None,
    };
    let (decision, origin) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::PaperConceptReview {
                input: Box::new(Input::Draft {
                    draft: draft.clone(),
                }),
            },
        )
        .await
        .unwrap();
    assert_eq!(origin.package_key, key);
    assert_eq!(
        decision,
        WorkflowDecisionV1::PaperConceptReview {
            decision: Decision::Draft {
                result: Ok(protocol::request_hash(&draft).unwrap())
            }
        }
    );
    let mut invalid = draft;
    invalid.candidate_revision = 0;
    let (decision, _) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::PaperConceptReview {
                input: Box::new(Input::Draft { draft: invalid }),
            },
        )
        .await
        .unwrap();
    let WorkflowDecisionV1::PaperConceptReview {
        decision: Decision::Draft { result: Err(error) },
    } = decision
    else {
        panic!("draft rejection")
    };
    assert_eq!(error.to_string(), "概念条目审查参数无效");
    assert_eq!(counts(&f), (0, 0, 0, 0, 0, 0));
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_items_accept_concepts_aliases_definitions_roles_then_relation_replay_and_reopen()
 {
    let (mut f, candidate) = setup(payload()).await;
    let premature = request(
        &candidate,
        "relation-1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::AssertRelation {
            subject_concept_id: "unknown-a".into(),
            object_concept_id: "unknown-b".into(),
            project_id: None,
        }),
    );
    let error = decide(&f, premature).await.unwrap_err();
    assert!(error.to_string().contains("关系两端必须先完成概念消歧"));
    assert_eq!(counts(&f), (0, 0, 0, 0, 0, 0));
    let first_request = request(
        &candidate,
        "concept-1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let first = decide(&f, first_request.clone()).await.unwrap();
    assert_eq!(first.review_state, "unreviewed");
    let first = target(&first, "concept-1");
    let second_request = request(
        &candidate,
        "concept-2",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let second = decide(&f, second_request.clone()).await.unwrap();
    let second = target(&second, "concept-2");
    let relation_request = request(
        &candidate,
        "relation-1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::AssertRelation {
            subject_concept_id: first,
            object_concept_id: second,
            project_id: None,
        }),
    );
    let complete = decide(&f, relation_request.clone()).await.unwrap();
    assert_eq!(complete.review_state, "accepted_unchanged");
    assert_eq!(counts(&f), (4, 2, 2, 1, 2, 3));
    let dimensions:(String,String,String,String,i64,String)=f.store.connection().unwrap().query_row("select paper_function,novelty_status,explanation_status,source_candidate_id,source_candidate_revision,source_local_concept_id from paper_concept_assertion_revisions where source_local_concept_id='concept-1'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        dimensions,
        (
            "focal".into(),
            "introduced_by_paper".into(),
            "explained_in_paper".into(),
            candidate.id.clone(),
            1,
            "concept-1".into()
        )
    );
    let requests = [first_request, second_request, relation_request];
    for r in &requests {
        decide(&f, r.clone()).await.unwrap();
    }
    assert_eq!(counts(&f), (4, 2, 2, 1, 2, 3));
    f = reopen_actual_fixture(f, false).await;
    for r in requests {
        decide(&f, r).await.unwrap();
    }
    assert_eq!(counts(&f), (4, 2, 2, 1, 2, 3));
    assert_eq!(
        ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&candidate.id)
            .unwrap()
            .review_state,
        "accepted_unchanged"
    );
    close(f).await;
}
