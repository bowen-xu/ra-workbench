//! Original citation history through the actual package, Core writer and normal reopen.
use super::*;
use crate::ai::review_support::{
    ReviewFacts, current_payload_source, record_citation_assessment_with,
    validate_citation_readiness,
};
use research_workbench_native_sdk::{
    candidate_contracts::CandidatePayload,
    candidate_review::contracts::{AiCandidate, AiCitationAssessmentDraft},
};

pub(super) async fn generated() -> (Fixture, String, AiCandidate) {
    let (f, key, credentials, draft) = setup();
    let run = admit(&f, &key, &credentials, draft).await;
    let signal = CancellationSignal::default();
    let (host, _) = host::Host::new(&f, credentials, run.clone(), signal.clone(), payload());
    execute_research_task(&host, &run, &signal).await.unwrap();
    let candidate = {
        let db = f.store.connection().unwrap();
        let id: String = db
            .query_row(
                "select id from ai_candidates where run_id=?1",
                [&run.id],
                |row| row.get(0),
            )
            .unwrap();
        ReviewFacts::new(&db).candidate(&id).unwrap()
    };
    drop(host);
    (f, key, candidate)
}
fn decoded(candidate: &AiCandidate) -> CandidatePayload {
    let (value, _) = current_payload_source(candidate).unwrap();
    serde_json::from_value(value).unwrap()
}
fn draft(candidate: &AiCandidate, statement: &str, key: &str) -> AiCitationAssessmentDraft {
    AiCitationAssessmentDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: candidate.current_revision,
        statement_id: statement.into(),
        evidence_item_id: candidate.evidence_bundle.items[0].id.clone(),
        expected_assessment_revision: 0,
        semantic_relation: "supports".into(),
        rationale: "  Synthetic source reviewed.  ".into(),
        idempotency_key: key.into(),
    }
}
async fn record(f: &Fixture, key: &str, draft: AiCitationAssessmentDraft) -> AiCandidate {
    f.service
        .record_candidate_citation_assessment(f.store.clone(), key.into(), draft)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_citation_history_replay_and_normal_reopen() {
    let (mut f, key, candidate) = generated().await;
    let payload = decoded(&candidate);
    let statements: Vec<_> = payload
        .citation_references()
        .into_iter()
        .map(|reference| reference.statement_id)
        .collect();
    assert!(!statements.is_empty());
    assert!(validate_citation_readiness(&candidate, &payload, None).is_err());
    let mut current = candidate.clone();
    for (index, statement) in statements.iter().enumerate() {
        let request = draft(
            &candidate,
            statement,
            &format!("reference-assessment-{index}"),
        );
        current = record(&f, &key, request.clone()).await;
        let replay = record(&f, &key, request.clone()).await;
        assert_eq!(replay, current);
        let mut conflict = request;
        conflict.semantic_relation = "unsupported".into();
        let error = f
            .service
            .record_candidate_citation_assessment(f.store.clone(), key.clone(), conflict)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("引用判断幂等键已被不同参数使用"));
    }
    validate_citation_readiness(&current, &payload, None).unwrap();
    assert_eq!(current.citation_assessments.len(), statements.len());
    assert!(
        current
            .citation_assessments
            .iter()
            .all(|value| value.rationale == "Synthetic source reviewed.")
    );
    let mut negative = draft(&candidate, &statements[0], "reference-negative");
    negative.expected_assessment_revision = 1;
    negative.semantic_relation = "unsupported".into();
    current = record(&f, &key, negative).await;
    assert!(
        validate_citation_readiness(&current, &payload, None)
            .unwrap_err()
            .to_string()
            .contains("反对或不支持")
    );
    let mut positive = draft(&candidate, &statements[0], "reference-positive");
    positive.expected_assessment_revision = 2;
    current = record(&f, &key, positive).await;
    validate_citation_readiness(&current, &payload, None).unwrap();
    assert_eq!(current.citation_assessments.len(), statements.len() + 2);
    assert!(f.store.generation_policy_source().is_none());
    f = reopen_actual_fixture(f, false).await;
    let reopened = ReviewFacts::new(&f.store.connection().unwrap())
        .candidate(&candidate.id)
        .unwrap();
    assert_eq!(reopened, current);
    validate_citation_readiness(&reopened, &decoded(&reopened), None).unwrap();
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_citation_errors_preserve_original_order_and_write_nothing() {
    let (f, key, candidate) = generated().await;
    let payload = decoded(&candidate);
    let statement = payload.citation_references()[0].statement_id.clone();
    let valid = draft(&candidate, &statement, "reference-errors");
    let mut invalid = vec![valid.clone(); 5];
    invalid[0].candidate_revision += 1;
    invalid[1].statement_id = "missing-statement".into();
    invalid[2].evidence_item_id = "missing-evidence".into();
    invalid[3].expected_assessment_revision = 1;
    invalid[4].semantic_relation = "invalid-relation".into();
    for request in invalid {
        let expected = {
            let db = f.store.connection().unwrap();
            record_citation_assessment_with::<crate::ai::error::AiError>(
                &db,
                request.clone(),
                |value| Ok(decoded(value)),
            )
            .unwrap_err()
        };
        let actual = f
            .service
            .record_candidate_citation_assessment(f.store.clone(), key.clone(), request)
            .await
            .unwrap_err();
        assert_eq!(actual.code(), expected.code());
        assert_eq!(actual.to_string(), expected.to_string());
    }
    let unchanged = ReviewFacts::new(&f.store.connection().unwrap())
        .candidate(&candidate.id)
        .unwrap();
    assert_eq!(unchanged, candidate);
    f.store.connection().unwrap().execute_batch(
        "create temp trigger reject_reference_assessment before insert on ai_citation_assessments begin select raise(abort,'synthetic private assessment'); end;"
    ).unwrap();
    let error = f
        .service
        .record_candidate_citation_assessment(f.store.clone(), key.clone(), valid.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(!error.to_string().contains("synthetic private assessment"));
    assert_eq!(
        ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&candidate.id)
            .unwrap(),
        candidate
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_reference_assessment")
        .unwrap();
    assert_eq!(record(&f, &key, valid).await.citation_assessments.len(), 1);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
