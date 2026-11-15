//! Faults retain original atomic knowledge/review writes and recovery fencing.
use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_items_each_write_fault_rolls_back_knowledge_history_and_review()
{
    for (table, event) in [
        ("terms", "insert"),
        ("research_object_revisions", "insert"),
        ("concepts", "insert"),
        ("concept_terms", "insert"),
        ("term_usages", "insert"),
        ("semantic_representations", "insert"),
        ("ai_suggestion_item_decisions", "insert"),
        ("ai_candidates", "update of review_state"),
    ] {
        let (f, _, credentials, task) = super::super::super::suggestion_generation::setup();
        let candidate = qualify(
            &f,
            &generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await,
        )
        .await;
        let before = counts(&f);
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger reject_knowledge_review before {event} on {table} begin select raise(abort,'synthetic private knowledge'); end;")).unwrap();
        let draft = request(
            &candidate,
            "concept_1",
            "accept_unchanged",
            Some(AiSuggestionResolutionDraft::CreateConcept),
        );
        let error = decide(&f, draft.clone()).await.unwrap_err();
        assert_eq!(error.code(), "storage_error", "{table} {error}");
        let visible =
            serde_json::to_string(&crate::ai::error::AiCommandError::from(error)).unwrap();
        assert!(!visible.contains("synthetic private knowledge"));
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
            .execute_batch("drop trigger reject_knowledge_review")
            .unwrap();
        decide(&f, draft).await.unwrap();
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_items_unknown_commit_and_written_prefix_rollback_fence_and_reopen()
 {
    for commit in [true, false] {
        let (f, _, credentials, task) = super::super::super::suggestion_generation::setup();
        let candidate = qualify(
            &f,
            &generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await,
        )
        .await;
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
            "concept_1",
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
async fn actual_package_suggestion_items_preserve_citation_before_inspection_and_resolution_errors()
{
    let (f, _, credentials, task) = super::super::super::suggestion_generation::setup();
    let candidate = generated(&f, &credentials, &task, payload("concept_suggestion_v1")).await;
    let draft = request(
        &candidate,
        "concept_1",
        "accept_unchanged",
        Some(AiSuggestionResolutionDraft::CreateConcept),
    );
    let expected = "接受前，每条候选陈述至少需要一条人工确认支持的引用";
    let actual = decide(&f, draft.clone()).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    let mut invalid = draft.clone();
    invalid.resolution = Some(AiSuggestionResolutionDraft::AttachDefinition {
        concept_id: "missing".into(),
    });
    let expected = "建议条目目标消歧方式无效";
    let actual = decide(&f, invalid).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    let _ = qualify_with_inspection(&f, &candidate, false).await;
    let expected = "原样接受前必须先打开该建议的至少一条来源";
    let actual = decide(&f, draft).await.unwrap_err();
    assert!(actual.to_string().contains(&expected.to_string()));
    assert_eq!(counts(&f).5, 0);
    close(f).await;
}
