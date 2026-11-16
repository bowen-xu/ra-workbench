//! Core retains real provider/scope facts while the package supplies original policy.
use super::*;
use crate::ai::{JsonCredentialStore, reading_contracts::LlmModelSelectionDraft};
use research_workbench_native_sdk::analysis_workflow::overview_task::OverviewTaskInputV1;

fn stable_preview(preview: &crate::ai::generation_support::AiTaskPreview) -> Value {
    let mut value = serde_json::to_value(preview).unwrap();
    for scope in value["scopeItems"].as_array_mut().unwrap() {
        // Original previews allocate ephemeral row IDs and SQL clock values.
        scope.as_object_mut().unwrap().remove("id");
        scope.as_object_mut().unwrap().remove("createdAt");
    }
    value
}

pub(in crate::runtime_plugins::literature_trace::service::context_metadata::tests) fn seeded_task(
    f: &Fixture,
) -> (JsonCredentialStore, OverviewTaskInputV1) {
    paper_preparation::seed(f);
    let reference = analysis_template::adopted(&f.store);
    let credentials =
        JsonCredentialStore::from_path(f._data.path().join("synthetic-empty-credentials.json"));
    {
        let db = f.store.connection().unwrap();
        db.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('deepseek',1,'https://api.deepseek.com',60000,'now') on conflict(provider_id) do update set enabled=1",[]).unwrap();
        db.execute("insert into full_text_index_jobs(attachment_id,state,page_count,updated_at) values('selected','completed',1,'index-v1') on conflict(attachment_id) do update set state='completed',page_count=1,updated_at='index-v1'",[]).unwrap();
        db.execute("insert into attachment_page_texts(attachment_id,page_index,page_text,indexed_at) values('selected',0,'Research question. Methods. Results. Synthetic page Ω','index-v1')",[]).unwrap();
    }
    let input = OverviewTaskInputV1 {
        paper_id: "paper".into(),
        attachment_ids: vec!["selected".into()],
        selection_revision: 0,
        template: Some(reference),
        model_selection: Some(LlmModelSelectionDraft {
            provider_id: "deepseek".into(),
            model_id: "synthetic-text-model".into(),
            reasoning_effort: None,
        }),
    };
    (credentials, input)
}

pub(super) fn suggestion_task(
    f: &Fixture,
    kind: &str,
) -> (
    JsonCredentialStore,
    crate::ai::generation_support::AiTaskSpecDraft,
) {
    let (credentials, input) = seeded_task(f);
    let mut draft = protocol::overview_task(&input);
    draft.task_kind = "paper_suggestions".into();
    draft.paper_template_ref = None;
    draft.overview_profile_selection_revision = None;
    draft.suggestion_candidate_kind = Some(kind.into());
    draft.user_goal = "Synthetic suggestions Ω".into();
    (credentials, draft)
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_preview_and_admission_keep_original_four_kinds_and_shared_identity()
 {
    for kind in research_workbench_native_sdk::generation::SUGGESTION_CANDIDATE_KINDS {
        let (catalog, key) = actual_package();
        let mut f = Fixture::new();
        let (credentials, mut draft) = suggestion_task(&f, kind);
        let expected =
            prepared_preview(&f.store.connection().unwrap(), &credentials, &draft).unwrap();
        f.bind_actual_package(catalog, key.clone());
        assert!(f.store.generation_policy_source().is_none());
        assert!(f.store.paper_generation_source().is_none());
        let preview = f
            .service
            .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft.clone())
            .await
            .unwrap();
        assert_eq!(stable_preview(&preview), stable_preview(&expected));
        assert_eq!(preview.suggestion_candidate_kind.as_deref(), Some(kind));
        assert_eq!(preview.evidence_preview.len(), 1);
        assert_eq!(preview.evidence_preview[0].citation_key, "ev-page-1");
        draft.confirmed_scope_hash = Some(preview.scope_hash.clone());
        draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
        let run = f
            .service
            .admit_analysis_overview(f.store.clone(), credentials, key, draft, None)
            .await
            .unwrap();
        assert_eq!(run.scope_hash, preview.scope_hash);
        assert_eq!(run.suggestion_candidate_kind.as_deref(), Some(kind));
        assert_eq!(run.status, "queued");
        assert!(run.shared_conversation_id.is_some());
        assert!(run.shared_request_id.is_some());
        let db = f.store.connection().unwrap();
        let counts: (i64, i64, i64, i64) = db
            .query_row(
                "select (select count(*) from ai_runs),(select count(*) from ai_scope_items),
             (select count(*) from ai_suggestion_run_contexts),
             (select count(*) from agent_legacy_ai_run_bindings)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(counts, (1, 1, 1, 1));
        drop(db);
        f.service.shutdown().await.unwrap();
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_suggestion_admission_rolls_back_original_shared_binding_and_kind_context() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut draft) = suggestion_task(&f, "concept_suggestion_v1");
    f.bind_actual_package(catalog, key.clone());
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, draft.clone())
        .await
        .unwrap();
    draft.confirmed_scope_hash = Some(preview.scope_hash);
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let conversations_before: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("select count(*) from agent_conversations", [], |row| {
            row.get(0)
        })
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Insert {
                    table_name: "ai_suggestion_run_contexts"
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let error = f
        .service
        .admit_analysis_overview(
            f.store.clone(),
            credentials.clone(),
            key.clone(),
            draft.clone(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    let db = f.store.connection().unwrap();
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let counts: (i64, i64, i64, i64, i64) = db
        .query_row(
            "select (select count(*) from ai_runs),(select count(*) from ai_scope_items),
         (select count(*) from ai_suggestion_run_contexts),
         (select count(*) from agent_legacy_ai_run_bindings),
         (select count(*) from agent_conversations)",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(counts, (0, 0, 0, 0, conversations_before));
    assert!(db.is_autocommit());
    drop(db);
    f.service
        .admit_analysis_overview(f.store.clone(), credentials, key, draft, None)
        .await
        .unwrap();
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_overview_task_and_preview_match_original_core_scope_hash_and_evidence() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, input) = seeded_task(&f);
    let expected_task = protocol::overview_task(&input);
    let expected =
        prepared_preview(&f.store.connection().unwrap(), &credentials, &expected_task).unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.generation_policy_source().is_none());
    let mut task = f
        .service
        .build_analysis_overview_task(&key, input)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&task).unwrap(),
        serde_json::to_vec(&expected_task).unwrap()
    );
    let actual = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, task.clone())
        .await
        .unwrap();
    assert_eq!(stable_preview(&actual), stable_preview(&expected));
    assert_eq!(actual.evidence_preview.len(), 1);
    assert_eq!(actual.evidence_preview[0].citation_key, "ev-page-1");
    task.confirmed_scope_hash = Some(actual.scope_hash.clone());
    task.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let run = f
        .service
        .admit_analysis_overview(
            f.store.clone(),
            credentials.clone(),
            key.clone(),
            task.clone(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(run.scope_hash, actual.scope_hash);
    assert_eq!(run.provider_id, actual.provider_id);
    assert_eq!(run.model_id, actual.model_id);
    assert_eq!(run.status, "queued");
    assert!(run.shared_conversation_id.is_some());
    assert!(run.shared_request_id.is_some());
    assert_eq!(run.overview_profile, actual.overview_profile);
    let before: (i64, i64) = f.store.connection().unwrap().query_row("select (select count(*) from ai_runs),(select count(*) from agent_legacy_ai_run_bindings)", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
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
    let failed = f
        .service
        .admit_analysis_overview(
            f.store.clone(),
            credentials.clone(),
            key.clone(),
            task.clone(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(failed.code(), "storage_error");
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let after: (i64, i64) = f.store.connection().unwrap().query_row("select (select count(*) from ai_runs),(select count(*) from agent_legacy_ai_run_bindings)", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(
        after, before,
        "the original run and shared binding must roll back together"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=0 where provider_id='deepseek'",
            [],
        )
        .unwrap();
    let expected =
        prepared_preview(&f.store.connection().unwrap(), &credentials, &task).unwrap_err();
    let error = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials, &key, task)
        .await
        .unwrap_err();
    assert_eq!(error.code(), expected.code());
    assert_eq!(error.to_string(), expected.to_string());
    let db = f.store.connection().unwrap();
    let runs: i64 = db
        .query_row("select count(*) from ai_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(runs, 1);
    assert!(db.is_autocommit());
    drop(db);
    f.service.shutdown().await.unwrap();
}
