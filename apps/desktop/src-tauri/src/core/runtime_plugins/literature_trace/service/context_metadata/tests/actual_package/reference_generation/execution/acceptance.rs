//! Real Core and an independently built package close the reference review transaction.
use super::*;
use crate::ai::review_support::{ReviewFacts, current_payload_source};
use research_workbench_native_sdk::candidate_review::contracts::{
    AiCandidate, AiCandidateRevisionDraft, AiCitationAssessmentDraft, AiReviewDecisionDraft,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn draft(candidate: &AiCandidate, action: &str, key: &str) -> AiReviewDecisionDraft {
    AiReviewDecisionDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: candidate.current_revision,
        action: action.into(),
        rationale: "  Synthetic source checked.  ".into(),
        idempotency_key: key.into(),
        reading_unit_title: None,
        project_id: None,
    }
}
async fn qualify(f: &Fixture, key: &str, candidate: &AiCandidate) -> AiCandidate {
    let (value, _) = current_payload_source(candidate).unwrap();
    let payload: research_workbench_native_sdk::candidate_contracts::CandidatePayload =
        serde_json::from_value(value).unwrap();
    for (index, reference) in payload.citation_references().iter().enumerate() {
        f.service
            .record_candidate_citation_assessment(
                f.store.clone(),
                key.into(),
                AiCitationAssessmentDraft {
                    candidate_id: candidate.id.clone(),
                    candidate_revision: candidate.current_revision,
                    statement_id: reference.statement_id.clone(),
                    evidence_item_id: candidate.evidence_bundle.items[0].id.clone(),
                    expected_assessment_revision: 0,
                    semantic_relation: "supports".into(),
                    rationale: "Synthetic checked source.".into(),
                    idempotency_key: format!("qualify-{}-{index}", candidate.current_revision),
                },
            )
            .await
            .unwrap();
    }
    f.store.connection().unwrap().execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values(?1,?2,?3,?4,'now')", rusqlite::params![uuid::Uuid::new_v4().to_string(),candidate.id,candidate.current_revision,candidate.evidence_bundle.items[0].id]).unwrap();
    ReviewFacts::new(&f.store.connection().unwrap())
        .candidate(&candidate.id)
        .unwrap()
}
fn counts(f: &Fixture) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from reference_records),(select count(*) from reference_record_revisions),(select count(*) from ai_review_decisions)", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
}
async fn decide(f: &Fixture, key: &str, request: AiReviewDecisionDraft) -> AiCandidate {
    f.service
        .decide_analysis_candidate(f.store.clone(), key.into(), request)
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_acceptance_replay_fields_and_normal_reopen() {
    for edited in [false, true] {
        let (mut f, key, mut candidate) = super::review::generated().await;
        if edited {
            let (mut value, _) = current_payload_source(&candidate).unwrap();
            value["fields"][0]["value"] = json!("Human checked title Ω");
            candidate = f
                .service
                .revise_analysis_candidate(
                    f.store.clone(),
                    key.clone(),
                    AiCandidateRevisionDraft {
                        candidate_id: candidate.id.clone(),
                        expected_revision: 1,
                        payload: value,
                        change_reason: "Source correction".into(),
                    },
                )
                .await
                .unwrap();
        }
        candidate = qualify(&f, &key, &candidate).await;
        let before = counts(&f);
        f.store.connection().unwrap().execute_batch("insert into research_projects(id,title,summary,created_at) values('reference-acceptance-project','Synthetic review','','now'); insert into project_items(project_id,item_id,added_at) values('reference-acceptance-project','paper','now');").unwrap();
        let mut request = draft(
            &candidate,
            if edited {
                "accept_after_edit"
            } else {
                "accept_unchanged"
            },
            "accept-reference-once",
        );
        request.project_id = Some("reference-acceptance-project".into());
        let accepted = decide(&f, &key, request.clone()).await;
        assert_eq!(
            accepted.review_state,
            if edited {
                "accepted_after_edit"
            } else {
                "accepted_unchanged"
            }
        );
        assert_eq!(counts(&f), (before.0 + 1, before.1 + 1, before.2 + 1));
        let after = counts(&f);
        assert_eq!(decide(&f, &key, request.clone()).await, accepted);
        assert_eq!(counts(&f), after);
        let mut conflict = request;
        conflict.rationale = "Changed".into();
        let error = f
            .service
            .decide_analysis_candidate(f.store.clone(), key.clone(), conflict)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("审查幂等键已被不同"));
        let snapshot: Value = {
            let db = f.store.connection().unwrap();
            let (fields,provenance,identity,semantic,revision,snapshot):(String,String,String,String,i64,String)=db.query_row("select r.fields_json,r.provenance_kind,r.identity_state,r.semantic_state,r.current_revision,v.snapshot_json from reference_records r join reference_record_revisions v on v.reference_record_id=r.id",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).unwrap();
            assert_eq!(
                provenance,
                if edited {
                    "ai_accepted_after_edit"
                } else {
                    "ai_accepted_unchanged"
                }
            );
            assert_eq!(
                (identity.as_str(), semantic.as_str(), revision),
                ("unverified", "unreviewed", 1)
            );
            let fields: Value = serde_json::from_str(&fields).unwrap();
            assert_eq!(
                fields[0]["sourceKind"],
                if edited { "human" } else { "ai_inference" }
            );
            assert_eq!(fields[0]["confirmedByUser"], edited);
            if edited {
                assert_eq!(fields[0]["value"], "Human checked title Ω");
            }
            let target = accepted.decisions.last().unwrap();
            assert_eq!(
                target.target_object_kind.as_deref(),
                Some("reference_record")
            );
            assert!(target.target_object_id.is_some());
            assert_eq!(target.target_object_revision, Some(1));
            let snapshot: Value = serde_json::from_str(&snapshot).unwrap();
            assert_eq!(snapshot["acceptedFrom"]["candidateId"], candidate.id);
            assert_eq!(
                snapshot["acceptedFrom"]["candidateRevision"],
                candidate.current_revision
            );
            snapshot
        };
        assert!(f.store.generation_policy_source().is_none());
        f = reopen_actual_fixture(f, false).await;
        assert_eq!(
            ReviewFacts::new(&f.store.connection().unwrap())
                .candidate(&candidate.id)
                .unwrap(),
            accepted
        );
        let reopened: String = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select snapshot_json from reference_record_revisions",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&reopened).unwrap(), snapshot);
        assert_eq!(counts(&f), after);
        f.service.shutdown().await.unwrap();
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_defer_request_reject_and_validation_write_no_record() {
    let (f, key, mut candidate) = super::review::generated().await;
    let original = counts(&f);
    let accept = draft(&candidate, "accept_unchanged", "not-qualified");
    let expected = crate::ai::error::AiError::from(research_literature_library_native::candidate_review::validate_inspection(false).unwrap_err());
    let actual = f
        .service
        .decide_analysis_candidate(f.store.clone(), key.clone(), accept)
        .await
        .unwrap_err();
    assert_eq!(actual.code(), expected.code());
    assert!(actual.to_string().contains(&expected.to_string()));
    assert_eq!(counts(&f), original);
    let mut outside = draft(&candidate, "defer", "outside-project");
    outside.project_id = Some("missing-project".into());
    let error = f
        .service
        .decide_analysis_candidate(f.store.clone(), key.clone(), outside)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("项目"));
    assert_eq!(counts(&f), original);
    for (action, state) in [
        ("defer", "deferred"),
        ("request_more_evidence", "needs_more_evidence"),
        ("reject", "rejected"),
    ] {
        let request = draft(&candidate, action, action);
        candidate = decide(&f, &key, request.clone()).await;
        assert_eq!(candidate.review_state, state);
        assert_eq!(decide(&f, &key, request).await, candidate);
        assert_eq!((counts(&f).0, counts(&f).1), (original.0, original.1));
    }
    let count = counts(&f);
    let error = f
        .service
        .decide_analysis_candidate(
            f.store.clone(),
            key,
            draft(&candidate, "defer", "terminal-again"),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("审查已经结束"));
    assert_eq!(counts(&f), count);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_acceptance_rolls_back_every_original_write() {
    let (f, key, candidate) = super::review::generated().await;
    let candidate = qualify(&f, &key, &candidate).await;
    let before = counts(&f);
    let request = draft(&candidate, "accept_unchanged", "accept-after-fault");
    for event in [
        "insert on reference_records",
        "insert on reference_record_revisions",
        "insert on ai_review_decisions",
        "update of review_state on ai_candidates",
    ] {
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger fail_reference_acceptance before {event} begin select raise(abort,'synthetic private acceptance'); end;")).unwrap();
        let error = f
            .service
            .decide_analysis_candidate(f.store.clone(), key.clone(), request.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "storage_error");
        assert!(!error.to_string().contains("synthetic private acceptance"));
        assert_eq!(counts(&f), before);
        assert_eq!(
            ReviewFacts::new(&f.store.connection().unwrap())
                .candidate(&candidate.id)
                .unwrap(),
            candidate
        );
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_reference_acceptance")
            .unwrap();
    }
    assert_eq!(
        decide(&f, &key, request).await.review_state,
        "accepted_unchanged"
    );
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_acceptance_unknown_commit_or_rollback_fences_and_reopens() {
    for commit in [true, false] {
        let (f, key, candidate) = super::review::generated().await;
        let candidate = qualify(&f, &key, &candidate).await;
        let before = counts(&f);
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let db = f.store.connection().unwrap();
            db.execute_batch("create temp trigger fail_reference_acceptance before insert on ai_review_decisions begin select raise(abort,'synthetic private acceptance'); end;").unwrap();
            db.authorizer(Some(|context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Transaction {
                        operation: TransactionOperation::Rollback
                    }
                ) {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }));
        }
        let error = f
            .service
            .decide_analysis_candidate(
                f.store.clone(),
                key,
                draft(&candidate, "accept_unchanged", "uncertain-acceptance"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.to_string().contains(&candidate.run_id));
        assert!(error.to_string().contains(&candidate.id));
        assert!(!error.to_string().contains("synthetic private acceptance"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), before);
        assert_eq!(
            ReviewFacts::new(&f.store.connection().unwrap())
                .candidate(&candidate.id)
                .unwrap(),
            candidate
        );
        f.service.shutdown().await.unwrap();
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}
