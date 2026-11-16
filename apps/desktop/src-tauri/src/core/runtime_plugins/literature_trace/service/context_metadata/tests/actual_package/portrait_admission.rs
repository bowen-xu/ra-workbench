//! Original profile preview and complete run/conversation admission over the actual package.
use super::*;
use crate::ai::{JsonCredentialStore, generation_support::AiTaskSpecDraft};

fn draft(f: &Fixture) -> (JsonCredentialStore, AiTaskSpecDraft) {
    let (credentials, input) = analysis_overview::seeded_task(f);
    let mut draft = protocol::overview_task(&input);
    draft.task_kind = "paper_profile_classification".into();
    draft.paper_template_ref = None;
    draft.overview_profile_selection_revision = None;
    draft.user_goal = "Ω".repeat(2001);
    draft.pipeline_version = "paper-portrait-sources-v2".into();
    draft.prompt_version = "paper-portrait-catalog-v2".into();
    (credentials, draft)
}
fn stable_preview(preview: &crate::ai::generation_support::AiTaskPreview) -> Value {
    let mut value = serde_json::to_value(preview).unwrap();
    for scope in value["scopeItems"].as_array_mut().unwrap() {
        scope.as_object_mut().unwrap().remove("id");
        scope.as_object_mut().unwrap().remove("createdAt");
    }
    value
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row(
        "select (select count(*) from ai_runs),(select count(*) from ai_scope_items),
                (select count(*) from agent_legacy_ai_run_bindings),
                (select count(*) from agent_conversations),(select count(*) from paper_generation_conversations)",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).unwrap()
}
async fn admit(
    f: &Fixture,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
) -> Result<crate::ai::generation_support::AiRun, crate::ai::error::AiError> {
    f.service
        .admit_analysis_overview(
            f.store.clone(),
            credentials.clone(),
            f.key.clone(),
            draft.clone(),
            None,
        )
        .await
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_portrait_preview_and_admission_reuse_original_scope_and_conversation_rules()
{
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut draft) = draft(&f);
    let expected = prepared_preview(&f.store.connection().unwrap(), &credentials, &draft).unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.generation_policy_source().is_none());
    assert!(f.store.paper_generation_source().is_none());
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft.clone())
        .await
        .unwrap();
    assert_eq!(stable_preview(&preview), stable_preview(&expected));
    assert!(preview.evidence_preview.is_empty());
    draft.confirmed_scope_hash = Some(preview.scope_hash.clone());
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let first = admit(&f, &credentials, &draft).await.unwrap();
    assert_eq!(first.task_kind, "paper_profile_classification");
    assert_eq!(first.status, "queued");
    assert_eq!(first.scope_hash, preview.scope_hash);
    assert_eq!(first.provider_id, preview.provider_id);
    assert_eq!(first.model_id, preview.model_id);
    let conversation = first.shared_conversation_id.unwrap();
    let request = first.shared_request_id.unwrap();
    let before = counts(&f);
    let active = admit(&f, &credentials, &draft).await.unwrap_err();
    assert_eq!(active.code(), "invalid_input");
    assert!(active.to_string().contains("本文生成任务仍在运行"));
    assert_eq!(counts(&f), before);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update ai_runs set status='failed' where id=?1",
            [&first.id],
        )
        .unwrap();
    let second = admit(&f, &credentials, &draft).await.unwrap();
    assert_eq!(
        second.shared_conversation_id.as_deref(),
        Some(conversation.as_str())
    );
    assert_ne!(second.shared_request_id.as_deref(), Some(request.as_str()));
    {
        let db = f.store.connection().unwrap();
        db.execute(
            "update ai_runs set status='failed' where id=?1",
            [&second.id],
        )
        .unwrap();
        db.execute(
            "update agent_conversations set archived=1 where id=?1",
            [conversation.to_string()],
        )
        .unwrap();
    }
    let third = admit(&f, &credentials, &draft).await.unwrap();
    assert_ne!(
        third.shared_conversation_id.as_deref(),
        Some(conversation.as_str())
    );
    assert_eq!(
        counts(&f).4,
        1,
        "the original per-paper pointer is replaced"
    );
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_portrait_admission_rolls_back_conversation_run_scope_and_shared_binding() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut draft) = draft(&f);
    f.bind_actual_package(catalog, key.clone());
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft.clone())
        .await
        .unwrap();
    draft.confirmed_scope_hash = Some(preview.scope_hash);
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let before = counts(&f);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Insert {
                    table_name: "ai_scope_items"
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let error = admit(&f, &credentials, &draft).await.unwrap_err();
    assert_eq!(error.code(), "storage_error");
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(
        counts(&f),
        before,
        "all original admission writes must roll back together"
    );
    let run = admit(&f, &credentials, &draft).await.unwrap();
    assert_eq!(run.status, "queued");
    assert!(run.shared_conversation_id.is_some());
    assert!(run.shared_request_id.is_some());
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_portrait_sources_evidence_and_latest_use_original_owners_without_page_index_or_session_reads()
 {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut draft) = draft(&f);
    f.bind_actual_package(catalog, key.clone());
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft.clone())
        .await
        .unwrap();
    draft.confirmed_scope_hash = Some(preview.scope_hash);
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let run = admit(&f, &credentials, &draft).await.unwrap();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "attachment_page_texts" | "paper_ai_reading_sessions",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let sources = f
        .service
        .prepare_portrait_sources(
            f.store.clone(),
            key.clone(),
            "paper".into(),
            vec!["selected".into()],
        )
        .await
        .unwrap();
    assert!(sources.session.is_none());
    assert!(
        sources
            .context_items
            .iter()
            .all(|item| item.attachment_id == "selected")
    );
    assert!(!sources.context_items.is_empty());
    let bundle = f
        .service
        .portrait_evidence(f.store.clone(), key.clone(), run.clone())
        .await
        .unwrap();
    assert_eq!(bundle.validation_status, "valid");
    assert!(!bundle.items.is_empty());
    assert_eq!(bundle.run_id, run.id);
    f.service
        .validate_portrait_evidence(&key, &bundle)
        .await
        .unwrap();
    let again = f
        .service
        .portrait_evidence(f.store.clone(), key.clone(), run.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&bundle).unwrap(),
        serde_json::to_vec(&again).unwrap()
    );
    let latest = f
        .service
        .latest_portrait_run(f.store.clone(), key.clone(), "paper".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.id, run.id);
    assert_eq!(latest.shared_conversation_id, run.shared_conversation_id);
    assert_eq!(latest.shared_request_id, run.shared_request_id);
    let invalid = f
        .service
        .latest_portrait_run(f.store.clone(), key, " \t ".into())
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), "invalid_input");
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from ai_evidence_bundles where run_id=?1",
            [&run.id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from paper_context_snapshots", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0,
        "portrait evidence does not add an extraction snapshot"
    );
    drop(db);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_portrait_preview_keeps_original_invalid_input_order_and_messages() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, draft) = draft(&f);
    let mut retry = draft.clone();
    retry.base_candidate_id = Some("old-candidate".into());
    let mut complete_retry = retry.clone();
    complete_retry.base_candidate_revision = Some(1);
    complete_retry.overview_fields = vec!["question".into()];
    let mut invalid_suggestion = complete_retry.clone();
    invalid_suggestion.suggestion_candidate_kind = Some("concept_suggestions_v1".into());
    let mut invalid_budget = draft;
    invalid_budget.max_output_tokens = 0;
    let cases = vec![retry, complete_retry, invalid_suggestion, invalid_budget];
    let expected: Vec<_> = cases
        .iter()
        .map(|draft| {
            prepared_preview(&f.store.connection().unwrap(), &credentials, draft).unwrap_err()
        })
        .collect();
    f.bind_actual_package(catalog, key.clone());
    let before = counts(&f);
    for (draft, expected) in cases.into_iter().zip(expected) {
        let actual = f
            .service
            .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft)
            .await
            .unwrap_err();
        assert_eq!(actual.code(), expected.code());
        let actual = crate::ai::error::AiCommandError::from(actual);
        let expected = crate::ai::error::AiCommandError::from(expected);
        assert_eq!(actual.message(), expected.message());
    }
    assert_eq!(counts(&f), before);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
