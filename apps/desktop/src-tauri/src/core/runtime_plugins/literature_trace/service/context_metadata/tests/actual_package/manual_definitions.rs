//! Original manual definitions and immutable history through a real independent package.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::{
    manual::DefinitionClaimDraft, models::DefinitionClaim,
};
async fn fixture() -> Fixture {
    let f = evidence_matrix::fixture().await;
    f.store.connection().unwrap().execute_batch("insert into terms(id,text,normalized_text,language,created_at,updated_at) values('definition-term','Definition term','definition term','en','now','now');
    insert into concepts(id,label,explanation,created_at,updated_at) values('definition-concept','Definition concept','','now','now'),('definition-legacy','Definition legacy','','now','now');
    insert into concept_terms(concept_id,term_id,relation_kind,created_at) values('definition-concept','definition-term','preferred','now'),('definition-legacy','definition-term','preferred','now');
    insert into concept_identity_events(id,operation,survivor_concept_id,source_concept_ids_json,snapshot_json,created_at) values('definition-merge','merge','definition-concept','[\"definition-legacy\"]','{}','now');
    insert into concept_identity_mappings(source_concept_id,canonical_concept_id,merge_event_id,created_at) values('definition-legacy','definition-concept','definition-merge','now');").unwrap();
    f
}
fn draft() -> DefinitionClaimDraft {
    DefinitionClaimDraft {
        id: None,
        concept_id: "definition-legacy".into(),
        statement: " Definition Ω ".into(),
        scope_note: " Scope Ω ".into(),
        anchor_id: Some("matrix-first-anchor".into()),
        expected_revision: None,
        change_reason: None,
    }
}
async fn save(
    f: &Fixture,
    draft: DefinitionClaimDraft,
) -> Result<DefinitionClaim, crate::ai::error::AiError> {
    f.service
        .save_definition(f.store.clone(), f.key.clone(), draft)
        .await
}
fn counts(f: &Fixture) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from definition_claims),(select count(*) from definition_claim_evidence_links),(select count(*) from research_object_revisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_definition_create_edit_list_history_and_reopen() {
    let f = fixture().await;
    let created = save(&f, draft()).await.unwrap();
    assert_eq!(created.concept_id, "definition-concept");
    assert_eq!(created.statement, "Definition Ω");
    assert_eq!(created.scope_note, "Scope Ω");
    assert_eq!(created.provenance_kind, "human_original");
    assert_eq!(created.evidence_count, 1);
    let mut edit = draft();
    edit.id = Some(created.id.clone());
    edit.expected_revision = Some(1);
    edit.statement = " Changed Ω ".into();
    edit.change_reason = Some(" exact reason ".into());
    let edited = save(&f, edit).await.unwrap();
    // The original update retains its supplied concept ID, including an old identity.
    assert_eq!(edited.concept_id, "definition-legacy");
    assert_eq!(edited.revision, 2);
    assert_eq!(edited.evidence_count, 1);
    let listed = f
        .service
        .list_definitions(f.store.clone(), f.key.clone(), "definition-legacy".into())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&listed[0]).unwrap(),
        serde_json::to_value(&edited).unwrap()
    );
    let history = f
        .service
        .list_knowledge_revisions(
            f.store.clone(),
            f.key.clone(),
            "definition_claim".into(),
            edited.id.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        history.iter().map(|r| r.revision).collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(history[0].change_reason, " exact reason ");
    assert_eq!(history[0].snapshot["conceptId"], "definition-legacy");
    assert_eq!(history[1].snapshot["statement"], "Definition Ω");
    let before = counts(&f);
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(counts(&f), before);
    let after = f
        .service
        .list_knowledge_revisions(
            f.store.clone(),
            f.key.clone(),
            "definition_claim".into(),
            edited.id.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&history).unwrap()
    );
    assert_eq!(
        serde_json::to_value(
            &f.service
                .list_definitions(f.store.clone(), f.key.clone(), "definition-concept".into())
                .await
                .unwrap()[0]
        )
        .unwrap(),
        serde_json::to_value(&edited).unwrap()
    );
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_definition_original_errors_and_late_rollback() {
    let f = fixture().await;
    let mut missing = draft();
    missing.id = Some("missing".into());
    missing.statement.clear();
    assert_eq!(
        serde_json::to_value(crate::ai::knowledge_contracts::CaptureCommandError::from(
            save(&f, missing).await.unwrap_err()
        ))
        .unwrap()["message"],
        "找不到研究对象或来源锚点"
    );
    let mut invalid = draft();
    invalid.expected_revision = Some(1);
    invalid.concept_id = "missing".into();
    invalid.statement.clear();
    let error = save(&f, invalid).await.unwrap_err();
    assert_eq!(error.code(), "invalid_input");
    assert!(
        serde_json::to_value(crate::ai::knowledge_contracts::CaptureCommandError::from(
            error
        ))
        .unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("新建定义不能指定 expectedRevision")
    );
    assert_eq!(counts(&f), (0, 0, 0));
    let created = save(&f, draft()).await.unwrap();
    let mut stale = draft();
    stale.id = Some(created.id.clone());
    stale.expected_revision = Some(0);
    stale.statement.clear();
    let visible = serde_json::to_value(crate::ai::knowledge_contracts::CaptureCommandError::from(
        save(&f, stale).await.unwrap_err(),
    ))
    .unwrap();
    assert_eq!(visible["code"], "revision_conflict");
    assert_eq!(visible["nativeFailure"]["currentRevision"], 1);
    assert_eq!(visible["currentRevision"], 1);
    assert_eq!(visible["nativeFailure"]["packageKey"], f.key);
    assert!(visible["nativeFailure"]["requestId"].is_string());
    let before = counts(&f);
    for table in [
        "definition_claim_evidence_links",
        "research_object_revisions",
    ] {
        let db = f.store.connection().unwrap();
        db.execute_batch(&format!("create temp trigger manual_definition_failure before insert on {table} begin select raise(abort,'synthetic definition failure'); end;")).unwrap();
        drop(db);
        assert!(save(&f, draft()).await.is_err());
        assert_eq!(counts(&f), before);
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger manual_definition_failure")
            .unwrap();
    }
    let invalid_kind = f
        .service
        .list_knowledge_revisions(
            f.store.clone(),
            f.key.clone(),
            "unsupported".into(),
            "id".into(),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid_kind.code(), "invalid_input");
    let absent = f
        .service
        .list_definitions(f.store.clone(), f.key.clone(), "missing".into())
        .await
        .unwrap();
    assert!(absent.is_empty());
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_definition_unknown_commit_fences_and_reopens() {
    let f = fixture().await;
    f.store.connection().unwrap().commit_hook(Some(|| true));
    let error = save(&f, draft()).await.unwrap_err();
    assert_eq!(error.code(), "data_operation_outcome_unknown");
    assert!(f.store.connection().is_err());
    let f = reopen_actual_fixture(f, true).await;
    assert_eq!(counts(&f), (0, 0, 0));
    save(&f, draft()).await.unwrap();
    assert_eq!(counts(&f), (1, 1, 1));
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_definition_committed_read_failure_retains_the_write() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let f = fixture().await;
    let committed = Arc::new(AtomicBool::new(false));
    {
        let after_commit = Arc::clone(&committed);
        let db = f.store.connection().unwrap();
        db.commit_hook(Some(move || {
            after_commit.store(true, Ordering::SeqCst);
            false
        }));
        let after_commit = Arc::clone(&committed);
        db.authorizer(Some(move |context: AuthContext<'_>| {
            if after_commit.load(Ordering::SeqCst)
                && matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "definition_claims",
                        ..
                    }
                )
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    }
    let error = save(&f, draft()).await.unwrap_err();
    let visible = serde_json::to_value(crate::ai::knowledge_contracts::CaptureCommandError::from(
        error,
    ))
    .unwrap();
    assert_eq!(visible["nativeFailure"]["concepts"]["outcome"], "committed");
    assert!(visible["message"].as_str().unwrap().contains("已提交"));
    assert!(committed.load(Ordering::SeqCst));
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(counts(&f), (1, 1, 1));
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(counts(&f), (1, 1, 1));
    let listed = f
        .service
        .list_definitions(f.store.clone(), f.key.clone(), "definition-concept".into())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].statement, "Definition Ω");
    evidence_matrix::close(f).await;
}
