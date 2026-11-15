//! The original candidate editor persists a human revision through the actual Native package.
use super::*;
use crate::ai::review_support::{ReviewFacts, current_payload_source};
use research_workbench_native_sdk::{
    analysis_workflow::{
        candidate::{CandidateDecisionV1, CandidateInputV1},
        wire::{WorkflowDecisionV1, WorkflowInputV1},
    },
    candidate_review::contracts::AiCandidateRevisionDraft,
};
use sha2::{Digest, Sha256};

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_revision_keeps_history_canonical_bytes_and_normal_reopen() {
    let (mut f, key, mut candidate) = super::review::generated().await;
    candidate = f
        .service
        .record_candidate_citation_assessment(
            f.store.clone(),
            key.clone(),
            research_workbench_native_sdk::candidate_review::contracts::AiCitationAssessmentDraft {
                candidate_id: candidate.id.clone(),
                candidate_revision: 1,
                statement_id: "reference-field:title".into(),
                evidence_item_id: candidate.evidence_bundle.items[0].id.clone(),
                expected_assessment_revision: 0,
                semantic_relation: "supports".into(),
                rationale: "Original title reviewed.".into(),
                idempotency_key: "revision-original-assessment".into(),
            },
        )
        .await
        .unwrap();
    let (mut value, _) = current_payload_source(&candidate).unwrap();
    value["fields"][0]["value"] = json!("Human revised reference title Ω");
    let expected = prepared_revision(
        &f.store.connection().unwrap(),
        &candidate.run_id,
        &candidate.id,
        &candidate.candidate_kind,
        &candidate.evidence_bundle,
        value.clone(),
    )
    .unwrap();
    let draft = AiCandidateRevisionDraft {
        candidate_id: candidate.id.clone(),
        expected_revision: 1,
        payload: value,
        change_reason: "  Human reviewed source.  ".into(),
    };
    f.store.connection().unwrap().execute_batch("create temp trigger reject_reference_revision before update of current_revision on ai_candidates begin select raise(abort,'synthetic private revision'); end;").unwrap();
    let error = f
        .service
        .revise_analysis_candidate(f.store.clone(), key.clone(), draft.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(!error.to_string().contains("synthetic private revision"));
    assert_eq!(
        ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&candidate.id)
            .unwrap(),
        candidate
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_reference_revision")
        .unwrap();
    let revised = f
        .service
        .revise_analysis_candidate(f.store.clone(), key.clone(), draft.clone())
        .await
        .unwrap();
    assert_eq!(revised.id, candidate.id);
    assert_eq!(revised.current_revision, 2);
    assert_eq!(revised.review_state, "edited");
    assert_eq!(revised.revisions.len(), 2);
    assert_eq!(revised.revisions[0], candidate.revisions[0]);
    assert_eq!(revised.revisions[1].editor_kind, "human");
    assert_eq!(revised.revisions[1].parent_revision, Some(1));
    assert_eq!(revised.revisions[1].source_run_id, None);
    assert_eq!(revised.revisions[1].change_reason, "Human reviewed source.");
    let bytes: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select payload_json from ai_candidate_revisions where candidate_id=?1 and revision=2",
            [&candidate.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bytes, expected.canonical_json);
    assert_eq!(
        revised.revisions[1].payload_hash,
        format!("{:x}", Sha256::digest(bytes.as_bytes()))
    );
    assert_eq!(
        revised.original_payload_hash,
        candidate.original_payload_hash
    );
    assert_eq!(revised.citation_assessments, candidate.citation_assessments);
    let payload = serde_json::from_value(revised.revisions[1].payload.clone()).unwrap();
    assert!(
        crate::ai::review_support::validate_citation_readiness(
            &revised,
            &payload,
            Some("reference-field:title"),
        )
        .is_err()
    );
    let error = f
        .service
        .revise_analysis_candidate(f.store.clone(), key, draft)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("候选已产生新版本"));
    assert!(f.store.generation_policy_source().is_none());
    f = reopen_actual_fixture(f, false).await;
    assert_eq!(
        ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&candidate.id)
            .unwrap(),
        revised
    );
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_candidate_revision_keeps_original_core_field_hash_protection() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key.clone());
    let mut value = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":research_literature_library_native::analysis_workflow::candidate::GENERIC_OVERVIEW_FIELDS.iter().map(|field|json!({"field":field,"status":"not_found","statements":[]})).collect::<Vec<_>>()});
    let field = serde_json::to_string(&value["fields"][0]).unwrap();
    let hash = format!("{:x}", Sha256::digest(field.as_bytes()));
    let name = value["fields"][0]["field"].as_str().unwrap().to_owned();
    for changed in [false, true] {
        if changed {
            value["fields"][0]["status"] = json!("filled");
        }
        let payload: research_workbench_native_sdk::candidate_contracts::CandidatePayload =
            serde_json::from_value(value.clone()).unwrap();
        let input = CandidateInputV1::RevisionConfirmedFields {
            payload_cbor_base64: research_workbench_native_sdk::paper_context_wire::encode_payload(
                &payload,
            )
            .unwrap(),
            active_fields: vec![(name.clone(), hash.clone())],
        };
        let (decision, _) = f
            .service
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::Candidate {
                    input: Box::new(input),
                },
            )
            .await
            .unwrap();
        match decision {
            WorkflowDecisionV1::Candidate {
                decision: CandidateDecisionV1::Prepared { canonical_json },
            } if !changed => assert_eq!(canonical_json, serde_json::to_string(&payload).unwrap()),
            WorkflowDecisionV1::Candidate {
                decision: CandidateDecisionV1::Rejected { error },
            } if changed => assert_eq!(
                error.to_string(),
                format!("字段 {name} 已确认，不能再修改候选内容")
            ),
            _ => panic!("wrong original field protection result"),
        }
    }
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_revision_errors_keep_original_order_without_writes() {
    let (f, key, candidate) = super::review::generated().await;
    let (value, _) = current_payload_source(&candidate).unwrap();
    let base = AiCandidateRevisionDraft {
        candidate_id: candidate.id.clone(),
        expected_revision: 1,
        payload: value,
        change_reason: "Human correction".into(),
    };
    let mut invalid = vec![base.clone(); 5];
    invalid[0].change_reason = "Ω".repeat(501);
    invalid[0].candidate_id = "missing-candidate".into();
    invalid[1].expected_revision = 0;
    invalid[2].payload = serde_json::Value::Null;
    invalid[3].payload = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":research_literature_library_native::analysis_workflow::candidate::GENERIC_OVERVIEW_FIELDS.iter().map(|field|json!({"field":field,"status":"not_found","statements":[]})).collect::<Vec<_>>()});
    invalid[4].payload["fields"][0]["evidenceItemIds"] = json!(["outside-evidence"]);
    for request in invalid {
        let expected = {
            let mut db = f.store.connection().unwrap();
            crate::ai::review_support::revise_candidate_with::<crate::ai::error::AiError>(
                &mut db,
                request.clone(),
                false,
                |db, candidate, value| {
                    prepared_revision(
                        db,
                        &candidate.run_id,
                        &candidate.id,
                        &candidate.candidate_kind,
                        &candidate.evidence_bundle,
                        value,
                    )
                },
            )
            .unwrap_err()
        };
        let error = f
            .service
            .revise_analysis_candidate(f.store.clone(), key.clone(), request)
            .await
            .unwrap_err();
        assert_eq!(error.code(), expected.code());
        assert!(
            error.to_string().contains(&expected.to_string()),
            "expected={expected}, actual={error}"
        );
        assert_eq!(
            ReviewFacts::new(&f.store.connection().unwrap())
                .candidate(&candidate.id)
                .unwrap(),
            candidate
        );
    }
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_reference_revision_unknown_settlement_fences_and_normal_reopen_recovers() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for commit in [true, false] {
        let (mut f, key, candidate) = super::review::generated().await;
        let (mut value, _) = current_payload_source(&candidate).unwrap();
        value["fields"][0]["value"] = json!("Human revision with settlement fault");
        let request = AiCandidateRevisionDraft {
            candidate_id: candidate.id.clone(),
            expected_revision: 1,
            payload: value,
            change_reason: "Settlement fault".into(),
        };
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let db = f.store.connection().unwrap();
            db.execute_batch("create temp trigger reject_revision_pointer before update of current_revision on ai_candidates begin select raise(abort,'synthetic private revision'); end;").unwrap();
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
            .revise_analysis_candidate(f.store.clone(), key, request)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.to_string().contains(&candidate.run_id));
        assert!(error.to_string().contains(&candidate.id));
        assert!(
            error
                .to_string()
                .contains(if commit { "commit" } else { "rollback" })
        );
        assert!(!error.to_string().contains("synthetic private revision"));
        assert!(f.store.connection().is_err());
        f = reopen_actual_fixture(f, true).await;
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
