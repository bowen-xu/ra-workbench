//! Production analysis dispatch with the real Core Agent and an independent domain package.
use super::*;
use crate::{
    ai::{
        paper_analysis_contracts::PaperAnalysisStartDraft,
        paper_reading_contracts::PaperAiReadingImageAssetDraft,
    },
    runtime_plugins::literature_analysis::{
        commands::start::start_selected, manager::PaperAnalysisWorkflowManager,
    },
};

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_analysis_start_completes_original_agent_candidate_and_workflow() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, mut input) = analysis_overview::seeded_task(&f);
    let parser = crate::docling::preparation::test_support::Parser::new();
    parser.install(&f.store);
    let selection = input.model_selection.as_mut().unwrap();
    selection.provider_id = "openai".into();
    selection.model_id = "gpt-5.6-terra".into();
    f.store.connection().unwrap().execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('openai',1,'https://api.openai.com',60000,'now') on conflict(provider_id) do update set enabled=1",[]).unwrap();
    let prepared = crate::runtime_plugins::literature_trace::service::context_metadata::tests::actual_package::prepared_context(
        &f.store.connection().unwrap(),
        "paper",
        &["selected".into()],
    )
    .unwrap();
    let profile = prepared_template(
        &f.store.connection().unwrap(),
        "paper",
        input.template.as_ref().unwrap(),
    )
    .unwrap();
    let payload=json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1","paperTemplate":profile.paper_template,"fields":[{"field":"question","status":"not_found","statements":[]}]}).to_string();
    crate::agent::skills::bundled::install_defaults(f.store.agent_configuration_directory())
        .unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.paper_analysis_source().is_none());
    assert!(f.store.generation_policy_source().is_none());
    let (execution, wire) =
        analysis_docling_runner::setup_execution(&f, credentials.clone(), payload.clone());
    let manager = PaperAnalysisWorkflowManager::default();
    let draft = PaperAnalysisStartDraft {
        regenerate: false,
        client_request_id: "actual-start".into(),
        paper_id: input.paper_id,
        attachment_ids: input.attachment_ids,
        overview_profile_selection_revision: input.selection_revision,
        paper_template_ref: input.template,
        model_selection: input.model_selection,
        image_assets: vec![PaperAiReadingImageAssetDraft {
            context_item_id: prepared.image_requests[0].context_item_id.clone(),
            data_url: "data:image/png;base64,iVBORw0KGgpzeW50aGV0aWM=".into(),
        }],
        skill_source_ids: vec![],
        resource_inputs: vec![],
    };
    let workflow = start_selected(f.store.clone(), execution, manager.clone(), draft.clone())
        .await
        .unwrap();
    assert_eq!(
        workflow.status,
        "needs_review",
        "attempt failures {:?}",
        workflow
            .turns
            .iter()
            .flat_map(|turn| turn.attempts.iter())
            .map(|attempt| (&attempt.error_code, &attempt.error_message))
            .collect::<Vec<_>>()
    );
    assert_eq!(*wire.calls.lock().unwrap(), 3);
    assert_eq!(parser.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(manager.active_request("paper").unwrap(), None);
    assert_eq!(workflow.turns.len(), 1);
    let attempt = &workflow.turns[0].attempts[0];
    assert_eq!(attempt.status, "succeeded");
    assert_eq!(attempt.shared_turn_outcome.as_deref(), Some("completed"));
    assert_eq!(attempt.shared_terminal_status.as_deref(), Some("succeeded"));
    assert_eq!(attempt.candidate_id, attempt.shared_candidate_id);
    assert_eq!(attempt.ai_run_id, attempt.shared_ai_run_id);
    let db = f.store.connection().unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from ai_candidates where id=?1 and run_id=?2",
            rusqlite::params![attempt.candidate_id, attempt.ai_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert!(db.is_autocommit());
    drop(db);
    let current_profile = prepared_template(
        &f.store.connection().unwrap(),
        "paper",
        draft.paper_template_ref.as_ref().unwrap(),
    )
    .unwrap();
    assert!(current_profile.selection_revision > profile.selection_revision);
    // The original reuse rule permits a changed frozen template selection.
    protocol::validate_reuse(
        false,
        Some(&serde_json::to_string(&current_profile).unwrap()),
        Some(&workflow),
    )
    .unwrap();
    let (execution, rejected) = analysis_docling_runner::setup_execution(
        &f,
        credentials.clone(),
        "provider rejected".into(),
    );
    let mut regenerate = draft;
    regenerate.regenerate = true;
    let failed = start_selected(
        f.store.clone(),
        execution,
        manager.clone(),
        regenerate.clone(),
    )
    .await
    .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(
        failed.supersedes_workflow_id.as_deref(),
        Some(workflow.id.as_str())
    );
    assert_eq!(
        failed.turns[0].attempts[0].error_code.as_deref(),
        Some("rate_limited")
    );
    assert_eq!(
        failed.turns[0].attempts[0]
            .shared_terminal_status
            .as_deref(),
        Some("failed")
    );
    assert_eq!(
        *rejected.calls.lock().unwrap(),
        usize::from(research_agent_core::resilience::RetryPolicy::DEFAULT.max_attempts)
    );
    assert_eq!(manager.active_request("paper").unwrap(), None);
    let mut changed: Value = serde_json::from_str(&payload).unwrap();
    changed["fields"][0]["status"] = json!("filled");
    changed["fields"][0]["statements"] = json!([{
        "id":"question-disconnected", "text":"Synthetic continued question.",
        "epistemicRole":"source_paraphrase", "evidenceItemIds":["P1-T1"],
        "evidenceQuotes":[{"evidenceItemId":"P1-T1","quote":"Prepared evidence Ω"}]
    }]);
    let (execution, paused) =
        analysis_docling_runner::setup_execution(&f, credentials, changed.to_string());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    *paused.pause.lock().unwrap() = Some((started.clone(), release.clone()));
    regenerate.client_request_id = "actual-disconnected".into();
    let store = f.store.clone();
    let owner = manager.clone();
    let caller =
        tokio::spawn(async move { start_selected(store, execution, owner, regenerate).await });
    tokio::time::timeout(std::time::Duration::from_secs(30), started.notified())
        .await
        .unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(
        manager.active_request("paper").unwrap().as_deref(),
        Some("actual-disconnected")
    );
    assert!(
        manager
            .register_for_paper("other-request", "paper")
            .is_err()
    );
    assert!(f.store.connection().unwrap().is_autocommit());
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while manager.active_request("paper").unwrap().is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let completed = f
        .service
        .read_analysis_history(
            f.store.clone(),
            key.clone(),
            research_workbench_native_sdk::analysis_history::HistoryRequestV1::Latest {
                paper_id: "paper".into(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.status, "needs_review");
    assert_eq!(
        completed.turns[0].attempts[0]
            .shared_terminal_status
            .as_deref(),
        Some("succeeded")
    );
    assert_eq!(*paused.calls.lock().unwrap(), 3);
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
    drop(f.store);
    f.store = crate::storage::AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: f._data.path().into(),
            agent_configuration_directory: f._data.path().join("configuration"),
        },
        crate::storage::AppStorePorts::with_events(
            crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
            crate::storage::AppStoreSources::default(),
        ),
    )
    .unwrap();
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key.clone());
    let reopened = f
        .service
        .read_analysis_history(
            f.store.clone(),
            key,
            research_workbench_native_sdk::analysis_history::HistoryRequestV1::Get {
                workflow_id: completed.id.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(json!(reopened), json!(completed));
    f.service.shutdown().await.unwrap();
}
