//! Real shared-Agent generation, per-item acceptance and original knowledge writes.
use super::*;
use crate::ai::{
    CancellationSignal, JsonCredentialStore,
    generation_support::{AiTaskSpecDraft, execute_suggestions},
    review_support::{ReviewFacts, current_payload_source},
};
use research_workbench_native_sdk::{
    candidate_contracts::CandidatePayload,
    candidate_review::{
        contracts::{AiCandidate, AiCandidateRevisionDraft, AiCitationAssessmentDraft},
        items::AiSuggestionResolutionDraft,
    },
};
use rusqlite::params;
mod contracts;
mod failures;
mod waits;
async fn generated(
    f: &Fixture,
    credentials: &JsonCredentialStore,
    task: &AiTaskSpecDraft,
    value: Value,
) -> AiCandidate {
    let mut task = task.clone();
    task.suggestion_candidate_kind = Some(value["kind"].as_str().unwrap().into());
    let run = super::super::suggestion_generation::admit(f, &f.key, credentials, task).await;
    let signal = CancellationSignal::default();
    let (host, wire) = super::super::task_generation_host::Host::new(
        f,
        credentials.clone(),
        run.clone(),
        signal.clone(),
        value,
    );
    execute_suggestions(&host, &run, &signal).await.unwrap();
    assert_eq!(wire.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    super::super::suggestion_generation::assert_completed(f, &run.id);
    let db = f.store.connection().unwrap();
    let id: String = db
        .query_row(
            "select id from ai_candidates where run_id=?1",
            [&run.id],
            |r| r.get(0),
        )
        .unwrap();
    let candidate = ReviewFacts::new(&db).candidate(&id).unwrap();
    ReviewFacts::new(&db)
        .require_completed_turn(&candidate.id)
        .unwrap();
    candidate
}
fn payload(kind: &str) -> Value {
    super::super::suggestion_generation::payload(kind)
}
async fn qualify(f: &Fixture, candidate: &AiCandidate) -> AiCandidate {
    qualify_with_inspection(f, candidate, true).await
}
async fn qualify_with_inspection(
    f: &Fixture,
    candidate: &AiCandidate,
    inspect: bool,
) -> AiCandidate {
    let (value, _) = current_payload_source(candidate).unwrap();
    let decoded: CandidatePayload = protocol::decode_candidate_value(
        value,
        &candidate
            .evidence_bundle
            .items
            .iter()
            .map(|item| item.citation_key.clone())
            .collect(),
    )
    .unwrap();
    for reference in decoded.citation_references() {
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
                        rationale: "Checked synthetic source".into(),
                        idempotency_key: format!("qualification-{}", uuid::Uuid::new_v4()),
                    },
                )
                .await
                .unwrap();
        }
    }
    let db = f.store.connection().unwrap();
    if inspect {
        db.execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values(?1,?2,?3,?4,'now') on conflict(candidate_id,candidate_revision,evidence_item_id) do nothing",params![uuid::Uuid::new_v4().to_string(),candidate.id,candidate.current_revision,candidate.evidence_bundle.items[0].id]).unwrap();
    }
    ReviewFacts::new(&db).candidate(&candidate.id).unwrap()
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
        rationale: "Synthetic source checked Ω".into(),
        idempotency_key: format!("{}-{id}-{action}", candidate.id),
        resolution,
    }
}
async fn decide(
    f: &Fixture,
    draft: AiSuggestionItemDecisionDraft,
) -> Result<AiCandidate, crate::ai::error::AiError> {
    f.service
        .decide_suggestion_item(f.store.clone(), f.key.clone(), draft)
        .await
}
fn target(candidate: &AiCandidate, id: &str) -> String {
    candidate
        .suggestion_item_decisions
        .iter()
        .find(|decision| {
            decision.suggestion_id == id
                && matches!(
                    decision.action.as_str(),
                    "accept_unchanged" | "accept_after_edit"
                )
        })
        .unwrap()
        .target_object_id
        .clone()
        .unwrap()
}
async fn close(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from terms),(select count(*) from concepts),(select count(*) from term_usages),(select count(*) from definition_claims),(select count(*) from relation_assertions),(select count(*) from ai_suggestion_item_decisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_items_create_link_unresolved_define_relate_replay_and_reopen() {
    let (mut f, _, credentials, task) = super::super::suggestion_generation::setup();
    let before = counts(&f);
    let mut value = payload("concept_suggestion_v1");
    let mut second = value["suggestions"][0].clone();
    second["id"] = json!("concept_2");
    second["term"] = json!("混合专家");
    value["suggestions"].as_array_mut().unwrap().push(second);
    let candidate = qualify(&f, &generated(&f, &credentials, &task, value).await).await;
    let first_request = request(
        &candidate,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let partial = decide(&f, first_request.clone()).await.unwrap();
    assert_eq!(partial.review_state, "unreviewed");
    let first = target(&partial, "concept_1");
    let second_request = request(
        &partial,
        "concept_2",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let complete = decide(&f, second_request.clone()).await.unwrap();
    assert_eq!(complete.review_state, "accepted_unchanged");
    let second = target(&complete, "concept_2");
    let mut alias_value = payload("concept_suggestion_v1");
    alias_value["suggestions"][0]["term"] = json!("门控别名");
    let alias = qualify(&f, &generated(&f, &credentials, &task, alias_value).await).await;
    let alias_request = request(
        &alias,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::LinkConcept {
            concept_id: first.clone(),
        }),
    );
    let linked = decide(&f, alias_request.clone()).await.unwrap();
    assert_eq!(target(&linked, "concept_1"), first);
    let mut unresolved_value = payload("concept_suggestion_v1");
    unresolved_value["suggestions"][0]["term"] = json!("歧义语词");
    let unresolved = qualify(
        &f,
        &generated(&f, &credentials, &task, unresolved_value).await,
    )
    .await;
    let unresolved_request = request(
        &unresolved,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::Unresolved),
    );
    let unresolved = decide(&f, unresolved_request.clone()).await.unwrap();
    assert_eq!(
        unresolved.suggestion_item_decisions[0]
            .target_object_kind
            .as_deref(),
        Some("term_usage")
    );
    let unresolved_id = target(&unresolved, "concept_1");
    let concept: Option<String> = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select concept_id from term_usages where id=?1",
            [&unresolved_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(concept.is_none());
    let definition = qualify(
        &f,
        &generated(&f, &credentials, &task, payload("definition_suggestion_v1")).await,
    )
    .await;
    let definition_request = request(
        &definition,
        "definition_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::AttachDefinition {
            concept_id: first.clone(),
        }),
    );
    let definition = decide(&f, definition_request.clone()).await.unwrap();
    assert_eq!(
        definition.suggestion_item_decisions[0]
            .target_object_kind
            .as_deref(),
        Some("definition_claim")
    );
    let relation = qualify(
        &f,
        &generated(&f, &credentials, &task, payload("relation_suggestion_v1")).await,
    )
    .await;
    let relation_request = request(
        &relation,
        "relation_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::AssertRelation {
            subject_concept_id: first.clone(),
            object_concept_id: second.clone(),
            project_id: None,
        }),
    );
    let relation = decide(&f, relation_request.clone()).await.unwrap();
    assert_eq!(
        relation.suggestion_item_decisions[0]
            .target_object_kind
            .as_deref(),
        Some("relation_assertion")
    );
    let after = counts(&f);
    assert_eq!(
        (
            after.0 - before.0,
            after.1 - before.1,
            after.2 - before.2,
            after.3 - before.3,
            after.4 - before.4,
            after.5 - before.5
        ),
        (4, 2, 4, 1, 1, 6)
    );
    let (definitions,relations,revisions,representations):(i64,i64,i64,i64)=f.store.connection().unwrap().query_row("select (select count(*) from definition_claim_evidence_links),(select count(*) from relation_assertion_evidence_links),(select count(*) from research_object_revisions where editor_kind='ai_review'),(select count(*) from semantic_representations where subject_kind='concept')",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!((definitions, relations), (1, 1));
    assert!(revisions >= 13);
    assert!(representations >= 3);
    let requests = [
        first_request,
        second_request,
        alias_request,
        unresolved_request,
        definition_request,
        relation_request,
    ];
    for draft in &requests {
        decide(&f, draft.clone()).await.unwrap();
    }
    assert_eq!(counts(&f), after);
    f = reopen_actual_fixture(f, false).await;
    for draft in requests {
        decide(&f, draft).await.unwrap();
    }
    assert_eq!(counts(&f), after);
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_items_human_revision_and_attention_decisions_keep_original_provenance()
 {
    let (f, _, credentials, task) = super::super::suggestion_generation::setup();
    let candidate = generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await;
    let (mut value, _) = current_payload_source(&candidate).unwrap();
    value["suggestions"][0]["explanation"] = json!("Human correction Ω");
    let candidate = f
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
    let candidate = qualify(&f, &candidate).await;
    let accepted = decide(
        &f,
        request(
            &candidate,
            "concept_1",
            "accept_after_edit",
            Some(AiSuggestionResolutionDraft::CreateConcept),
        ),
    )
    .await
    .unwrap();
    assert_eq!(accepted.review_state, "accepted_after_edit");
    let (origin, body): (String, String) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select provenance_kind,explanation from concepts where id=?1",
            [target(&accepted, "concept_1")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(origin, "ai_accepted_after_edit");
    assert_eq!(body, "Human correction Ω");
    let attention = generated(&f, &credentials, &task, payload("attention_suggestion_v1")).await;
    let before = counts(&f);
    let error = decide(
        &f,
        request(
            &attention,
            "attention_1",
            "accept_unchanged",
            Some(AiSuggestionResolutionDraft::CreateConcept),
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("建议条目审查动作无效"));
    assert_eq!(counts(&f), before);
    let deferred = decide(&f, request(&attention, "attention_1", "defer", None))
        .await
        .unwrap();
    assert_eq!(deferred.review_state, "deferred");
    let rejected = decide(&f, request(&attention, "attention_1", "reject", None))
        .await
        .unwrap();
    assert_eq!(rejected.review_state, "rejected");
    let after = counts(&f);
    assert_eq!(
        (after.0, after.1, after.2, after.3, after.4),
        (before.0, before.1, before.2, before.3, before.4)
    );
    close(f).await;
}
