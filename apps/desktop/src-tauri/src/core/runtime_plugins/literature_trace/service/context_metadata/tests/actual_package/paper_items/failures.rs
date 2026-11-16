//! Paper-role rows retain the original atomic knowledge and review settlement.
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_items_late_alias_definition_and_role_faults_rollback_all_rows() {
    for (table, when) in [
        ("concept_terms", "when new.relation_kind='alias'"),
        ("definition_claims", ""),
        ("paper_concept_assertions", ""),
        ("paper_concept_assertion_revisions", ""),
    ] {
        let (f, candidate) = setup(payload()).await;
        let before = counts(&f);
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger fail_paper_role before insert on {table} {when} begin select raise(abort,'synthetic private paper-role'); end")).unwrap();
        let draft = request(
            &candidate,
            "concept-1",
            "accept_unchanged",
            Some(AiSuggestionResolutionDraft::CreateConcept),
        );
        let error = decide(&f, draft.clone()).await.unwrap_err();
        assert_eq!(error.code(), "storage_error", "{table}");
        let visible =
            serde_json::to_string(&crate::ai::error::AiCommandError::from(error)).unwrap();
        assert!(!visible.contains("synthetic private paper-role"));
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
            .execute_batch("drop trigger fail_paper_role")
            .unwrap();
        decide(&f, draft).await.unwrap();
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_items_unknown_commit_and_written_prefix_rollback_fence_and_reopen() {
    for commit in [true, false] {
        let (f, candidate) = setup(payload()).await;
        let before = counts(&f);
        let written = Arc::new(AtomicBool::new(false));
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let observe = Arc::clone(&written);
            let db = f.store.connection().unwrap();
            db.update_hook(Some(
                move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                    if action == rusqlite::hooks::Action::SQLITE_INSERT && table == "terms" {
                        observe.store(true, Ordering::SeqCst);
                    }
                },
            ));
            let armed = Arc::clone(&written);
            db.authorizer(Some(move |context: AuthContext<'_>| {
                if armed.load(Ordering::SeqCst)
                    && matches!(
                        context.action,
                        AuthAction::Transaction {
                            operation: TransactionOperation::Rollback
                        }
                    )
                {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }));
        }
        let draft = request(
            &candidate,
            "concept-1",
            "accept_unchanged",
            Some(AiSuggestionResolutionDraft::CreateConcept),
        );
        let error = decide(&f, draft.clone()).await.unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        if !commit {
            assert!(written.load(Ordering::SeqCst));
        }
        assert!(error.to_string().contains(&candidate.id));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), before);
        decide(&f, draft).await.unwrap();
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_items_unresolved_edited_link_and_nonaccepting_actions_keep_roles() {
    use research_workbench_native_sdk::candidate_review::contracts::AiCandidateRevisionDraft;
    let (f, candidate) = setup(payload()).await;
    let unresolved = decide(
        &f,
        request(
            &candidate,
            "concept-1",
            "accept_unchanged",
            Some(AiSuggestionResolutionDraft::Unresolved),
        ),
    )
    .await
    .unwrap();
    let decision = unresolved
        .suggestion_item_decisions
        .iter()
        .find(|d| d.suggestion_id == "concept-1")
        .unwrap();
    assert_eq!(decision.target_object_kind.as_deref(), Some("term_usage"));
    assert_eq!(counts(&f), (1, 0, 0, 0, 1, 1));
    let deferred = decide(&f, request(&candidate, "concept-2", "defer", None))
        .await
        .unwrap();
    assert_eq!(deferred.review_state, "deferred");
    let rejected = decide(&f, request(&candidate, "concept-2", "reject", None))
        .await
        .unwrap();
    assert_eq!(rejected.review_state, "unreviewed");
    assert_eq!(counts(&f), (1, 0, 0, 0, 1, 3));
    close(f).await;
    let (mut f, candidate) = setup(payload()).await;
    let mut value = payload();
    value["concepts"][0]["explanation"] = json!("Human corrected explanation Ω");
    let edited = f
        .service
        .revise_analysis_candidate(
            f.store.clone(),
            f.key.clone(),
            AiCandidateRevisionDraft {
                candidate_id: candidate.id.clone(),
                expected_revision: 1,
                payload: value,
                change_reason: "Human paper role correction".into(),
            },
        )
        .await
        .unwrap();
    let edited = generic_reading::qualify(&f, &edited).await;
    let first = request(
        &edited,
        "concept-1",
        "accept_after_edit",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let accepted = decide(&f, first.clone()).await.unwrap();
    let id = target(&accepted, "concept-1");
    let linked = request(
        &edited,
        "concept-2",
        "accept_after_edit",
        Some(AiSuggestionResolutionDraft::LinkConcept {
            concept_id: id.clone(),
        }),
    );
    let accepted = decide(&f, linked.clone()).await.unwrap();
    assert_eq!(target(&accepted, "concept-2"), id);
    assert_eq!(counts(&f), (4, 1, 2, 0, 2, 2));
    let (source, body): (String, String) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select provenance_kind,explanation from concepts where id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(source, "ai_accepted_after_edit");
    assert_eq!(body, "Human corrected explanation Ω");
    f = reopen_actual_fixture(f, false).await;
    decide(&f, first).await.unwrap();
    decide(&f, linked).await.unwrap();
    assert_eq!(counts(&f), (4, 1, 2, 0, 2, 2));
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_items_keep_citation_inspection_role_and_shared_turn_guards() {
    use research_workbench_native_sdk::candidate_review::contracts::{
        AiCandidateRevisionDraft, AiCitationAssessmentDraft,
    };
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let candidate = generic_reading::seed(&f, payload());
    f.bind_actual_package(catalog, key);
    let draft = request(
        &candidate,
        "concept-1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let expected = "接受前，每条候选陈述至少需要一条人工确认支持的引用";
    let actual = decide(&f, draft.clone()).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    assert_eq!(counts(&f), (0, 0, 0, 0, 0, 0));
    let decoded = protocol::decode_candidate_value(
        payload(),
        &std::collections::HashSet::from(["ev-quote".into()]),
    )
    .unwrap();
    for reference in decoded.citation_references() {
        crate::ai::review_support::record_citation_assessment_with(
            &f.store.connection().unwrap(),
            AiCitationAssessmentDraft {
                candidate_id: candidate.id.clone(),
                candidate_revision: 1,
                statement_id: reference.statement_id.clone(),
                evidence_item_id: "legacy-item".into(),
                expected_assessment_revision: 0,
                semantic_relation: "supports".into(),
                rationale: "Synthetic checked source".into(),
                idempotency_key: reference.statement_id,
            },
            |_| Ok::<_, crate::ai::error::AiError>(decoded.clone()),
        )
        .unwrap();
    }
    let expected = "原样接受前必须先打开该概念或关系的至少一条来源";
    let actual = decide(&f, draft.clone()).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    assert!(expected.to_string().contains("至少一条来源"));
    let before = ReviewFacts::new(&f.store.connection().unwrap())
        .candidate(&candidate.id)
        .unwrap();
    let mut invalid = payload();
    invalid["concepts"][0]["paperFunction"] = json!("unknown-role");
    let error = f
        .service
        .revise_analysis_candidate(
            f.store.clone(),
            f.key.clone(),
            AiCandidateRevisionDraft {
                candidate_id: candidate.id.clone(),
                expected_revision: 1,
                payload: invalid,
                change_reason: "Invalid role must be rejected".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_input");
    assert_eq!(
        ReviewFacts::new(&f.store.connection().unwrap())
            .candidate(&candidate.id)
            .unwrap(),
        before
    );
    f.store.connection().unwrap().execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind,candidate_id) values('legacy-run',?1,?2,'paper_concept_map','legacy-candidate')",rusqlite::params![f.record.conversation_id.to_string(),f.record.request_id.to_string()]).unwrap();
    let error = decide(&f, request(&candidate, "concept-1", "defer", None))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "temporary_unavailable");
    assert_eq!(counts(&f), (0, 0, 0, 0, 0, 0));
    close(f).await;
}
