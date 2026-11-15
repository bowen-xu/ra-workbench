//! Preserve the original all-fields transaction, settlement fence and recovery.
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_generic_reading_every_write_boundary_rolls_back_all_fields() {
    for (table, event, condition) in [
        (
            "structured_reading_units",
            "insert",
            "when new.unit_kind='conclusion'",
        ),
        ("reading_unit_evidence_links", "insert", ""),
        ("research_object_revisions", "insert", ""),
        ("ai_review_decisions", "insert", ""),
        ("ai_candidates", "update of review_state", ""),
    ] {
        let (f, candidate) = setup(payload(true)).await;
        let candidate = qualify(&f, &candidate).await;
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger fail_historical_promotion before {event} on {table} {condition} begin select raise(abort,'synthetic private promotion'); end;")).unwrap();
        let request = draft(&candidate, "accept_unchanged");
        let error = decide(&f, request.clone()).await.unwrap_err();
        assert_eq!(error.code(), "storage_error");
        assert!(!error.to_string().contains("synthetic private promotion"));
        assert_eq!(counts(&f), (0, 0, 0, 0));
        assert_eq!(
            ReviewFacts::new(&f.store.connection().unwrap())
                .candidate(&candidate.id)
                .unwrap(),
            candidate
        );
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_historical_promotion")
            .unwrap();
        decide(&f, request).await.unwrap();
        assert_eq!(counts(&f), (2, 2, 2, 1));
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_generic_reading_unknown_commit_or_rollback_fences_and_recovers() {
    for commit in [true, false] {
        let (f, candidate) = setup(payload(true)).await;
        let candidate = qualify(&f, &candidate).await;
        if commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            let db = f.store.connection().unwrap();
            db.execute_batch("create temp trigger fail_historical_promotion before insert on ai_review_decisions begin select raise(abort,'synthetic private promotion'); end;").unwrap();
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
        let error = decide(&f, draft(&candidate, "accept_unchanged"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.to_string().contains(&candidate.id));
        assert!(!error.to_string().contains("synthetic private promotion"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), (0, 0, 0, 0));
        assert_eq!(
            ReviewFacts::new(&f.store.connection().unwrap())
                .candidate(&candidate.id)
                .unwrap(),
            candidate
        );
        decide(&f, draft(&candidate, "accept_unchanged"))
            .await
            .unwrap();
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_generic_reading_missing_inspection_citation_and_project_preserve_order() {
    let (f, candidate) = setup(payload(false)).await;
    let request = draft(&candidate, "accept_unchanged");
    let expected = "原样接受前必须先打开至少一条候选证据";
    let actual = decide(&f, request.clone()).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    assert_eq!(counts(&f), (0, 0, 0, 0));
    f.store.connection().unwrap().execute("insert into ai_evidence_inspections(id,candidate_id,candidate_revision,evidence_item_id,viewed_at) values('inspection','legacy-candidate',1,'legacy-item','now')",[]).unwrap();
    let expected = "接受前，每条候选陈述至少需要一条人工确认支持的引用";
    let actual = decide(&f, request.clone()).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    let candidate = qualify(&f, &candidate).await;
    let mut request = draft(&candidate, "accept_unchanged");
    request.project_id = Some("absent".into());
    let expected = "所选项目不包含该证据对应的文献";
    let actual = decide(&f, request).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    assert_eq!(counts(&f), (0, 0, 0, 0));
    close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_generic_reading_mapped_incomplete_turn_cannot_use_legacy_exception() {
    let (f, candidate) = setup(payload(false)).await;
    let candidate = qualify(&f, &candidate).await;
    f.store.connection().unwrap().execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind,candidate_id) values('legacy-run',?1,?2,'selection_explanation','legacy-candidate')",params![f.record.conversation_id.to_string(),f.record.request_id.to_string()]).unwrap();
    assert!(
        ReviewFacts::new(&f.store.connection().unwrap())
            .require_completed_turn(&candidate.id)
            .is_err()
    );
    let error = decide(&f, draft(&candidate, "accept_unchanged"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "temporary_unavailable");
    assert_eq!(counts(&f), (0, 0, 0, 0));
    close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_generic_reading_keeps_earlier_sql_before_later_unit_validation() {
    for fail_first in [true, false] {
        let mut value = payload(true);
        value["fields"][1]["field"] = json!("research_question");
        let (f, candidate) = setup(value).await;
        let candidate = qualify(&f, &candidate).await;
        let mut request = draft(&candidate, "accept_unchanged");
        request.reading_unit_title = Some("Ω".repeat(225));
        if fail_first {
            f.store.connection().unwrap().execute_batch("create temp trigger fail_first_unit before insert on structured_reading_units begin select raise(abort,'synthetic private first'); end;").unwrap();
        }
        let error = decide(&f, request).await.unwrap_err();
        assert_eq!(
            error.code(),
            if fail_first {
                "storage_error"
            } else {
                "invalid_input"
            }
        );
        if !fail_first {
            assert!(error.to_string().contains("AI 审查生成的阅读单元内容无效"));
        }
        assert_eq!(counts(&f), (0, 0, 0, 0));
        close(f).await;
    }
}
