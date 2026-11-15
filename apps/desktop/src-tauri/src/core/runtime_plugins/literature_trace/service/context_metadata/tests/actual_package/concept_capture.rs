//! Existing human capture through the production Core and current independent package.
use super::*;
use research_workbench_native_sdk::{concepts::NewConceptDetails, knowledge_promotion::capture::*};
fn details() -> NewConceptDetails {
    NewConceptDetails {
        label: " Human capture Ω ".into(),
        explanation: " Human explanation ".into(),
        disambiguation: "".into(),
        domain: "".into(),
        tradition: "".into(),
        period: "".into(),
        change_reason: None,
    }
}
fn draft() -> ConceptCaptureDraft {
    ConceptCaptureDraft {
        term_text: " Human term Ω ".into(),
        language: Some(" en ".into()),
        anchor_id: "matrix-first-anchor".into(),
        usage_text: "  ".into(),
        resolution: ConceptResolutionDraft::CreateNew {
            concept: details(),
            definition_statement: Some(" A source definition Ω ".into()),
        },
    }
}
async fn call(
    f: &Fixture,
    draft: ConceptCaptureDraft,
) -> Result<ConceptCaptureResult, crate::ai::error::AiError> {
    f.service
        .capture_concept(f.store.clone(), f.key.clone(), draft)
        .await
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from terms),(select count(*) from concepts),(select count(*) from term_usages),(select count(*) from definition_claims),(select count(*) from research_object_revisions),(select count(*) from semantic_representations)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_human_capture_create_link_unresolved_duplicate_and_reopen() {
    let mut f = evidence_matrix::fixture().await;
    f.store.connection().unwrap().execute("update source_anchors set quote_exact='  Original quote Ω  ' where id='matrix-first-anchor'",[]).unwrap();
    let first = call(&f, draft()).await.unwrap();
    let concept = first.concept.unwrap();
    assert_eq!(concept.label, "Human capture Ω");
    assert_eq!(first.term.text, "Human term Ω");
    assert_eq!(first.term.language, "en");
    assert_eq!(first.usage.usage_text, "  Original quote Ω  ");
    assert_eq!(first.usage.provenance_kind, "human_original");
    assert_eq!(
        first.definition_claim.unwrap().statement,
        "A source definition Ω"
    );
    let mut linked = draft();
    linked.term_text = " Alias Ω ".into();
    linked.usage_text = " Manual explanation ".into();
    linked.resolution = ConceptResolutionDraft::LinkExisting {
        concept_id: concept.id.clone(),
        definition_statement: Some("  ".into()),
    };
    let alias = call(&f, linked.clone()).await.unwrap();
    assert_eq!(alias.concept.as_ref().unwrap().id, concept.id);
    assert_eq!(alias.concept.as_ref().unwrap().revision, 2);
    assert_eq!(alias.usage.usage_text, "Manual explanation");
    assert!(alias.definition_claim.is_none());
    let before = counts(&f);
    let again = call(&f, linked.clone()).await.unwrap();
    assert_eq!(again.usage.id, alias.usage.id);
    assert_eq!(counts(&f), before);
    let mut unresolved = draft();
    unresolved.term_text = "Ambiguous Ω".into();
    unresolved.resolution = ConceptResolutionDraft::Unresolved;
    let unresolved = call(&f, unresolved).await.unwrap();
    assert!(unresolved.concept.is_none() && unresolved.definition_claim.is_none());
    assert!(unresolved.usage.concept_id.is_none());
    let snapshot = counts(&f);
    let saved = serde_json::to_value(&alias).unwrap();
    f = reopen_actual_fixture(f, false).await;
    assert_eq!(counts(&f), snapshot);
    assert_eq!(
        serde_json::to_value(call(&f, linked).await.unwrap()).unwrap(),
        saved
    );
    let provenance: Vec<(String, String)> = f
        .store
        .connection()
        .unwrap()
        .prepare(
            "select distinct editor_kind,change_reason from research_object_revisions order by change_reason",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(provenance.iter().all(|(editor, _)| editor == "human"));
    assert!(
        provenance
            .iter()
            .any(|(_, reason)| reason == "selection_capture_alias")
    );
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_human_capture_keeps_first_errors_and_rolls_back_late_failure() {
    use crate::ai::{error::AiError, knowledge_contracts::KnowledgeError};
    let f = evidence_matrix::fixture().await;
    let before = counts(&f);
    let mut missing = draft();
    missing.anchor_id = "missing".into();
    missing.term_text.clear();
    assert!(matches!(
        call(&f, missing).await,
        Err(AiError::Knowledge(KnowledgeError::NotFound))
    ));
    let mut empty = draft();
    empty.term_text.clear();
    assert_native_rejection(&f, call(&f, empty).await.unwrap_err());
    assert_eq!(counts(&f), before);
    f.store.connection().unwrap().execute_batch("create temp trigger capture_usage_failure before insert on term_usages begin select raise(abort,'synthetic usage write'); end;").unwrap();
    let error = call(&f, draft()).await.unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert_eq!(counts(&f), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger capture_usage_failure")
        .unwrap();
    call(&f, draft()).await.unwrap();
    let collision = call(&f, draft()).await.unwrap_err();
    assert_native_rejection(&f, collision);
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_human_capture_unknown_commit_or_prefix_rollback_fences_and_reopens() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    for commit in [true, false] {
        let f = evidence_matrix::fixture().await;
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
        let error = call(&f, draft()).await.unwrap_err();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        let visible = serde_json::to_value(crate::ai::error::AiCommandError::from(error)).unwrap();
        assert_eq!(visible["code"], "data_operation_outcome_unknown");
        if !commit {
            assert!(written.load(Ordering::SeqCst));
        }
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), before);
        call(&f, draft()).await.unwrap();
        evidence_matrix::close(f).await;
    }
}

fn assert_native_rejection(f: &Fixture, error: crate::ai::error::AiError) {
    assert_eq!(error.code(), "invalid_input");
    let visible = serde_json::to_value(crate::ai::error::AiCommandError::from(error)).unwrap();
    assert_eq!(visible["code"], "invalid_input");
    assert_eq!(visible["nativeFailure"]["packageKey"], f.key);
    assert!(visible["nativeFailure"]["instanceId"].is_string());
    assert!(visible["nativeFailure"]["requestId"].is_string());
}
