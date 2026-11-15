use super::*;
use serde_json::json;

#[tokio::test]
async fn preparation_freezes_exact_model_window_and_does_not_fetch_missing_metadata() {
    let mut f = fixture();
    let cache = f._directory.path().join("models.json");
    f.resolver.context_metadata = Some(cache.clone());
    assert_eq!(
        f.resolver
            .prepare(AgentHost::Writing, None)
            .await
            .unwrap()
            .context_window_tokens(),
        None
    );
    let catalog =
        |tokens| json!({"deepseek":{"models":{"global-model":{"limit":{"context":tokens}}}}});
    std::fs::write(&cache, catalog(128000).to_string()).unwrap();
    let frozen = f.resolver.prepare(AgentHost::Writing, None).await.unwrap();
    assert_eq!(frozen.context_window_tokens(), Some(128000));
    std::fs::write(&cache, catalog(256000).to_string()).unwrap();
    assert_eq!(frozen.context_window_tokens(), Some(128000));
    assert_eq!(
        f.resolver
            .prepare(AgentHost::Writing, None)
            .await
            .unwrap()
            .context_window_tokens(),
        Some(256000)
    );
    assert_eq!(
        f.resolver
            .prepare(
                AgentHost::Writing,
                selection("deepseek", "global-model-2026-10-09")
            )
            .await
            .unwrap()
            .context_window_tokens(),
        None
    );
    std::fs::write(&cache, "malformed").unwrap();
    assert_eq!(
        f.resolver
            .prepare(AgentHost::Writing, None)
            .await
            .unwrap()
            .context_window_tokens(),
        None
    );
}

#[test]
fn cached_execution_limits_reject_ambiguous_provider_aliases_and_unknown_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("models.json");
    let catalog = json!({"alibaba":{"models":{"qwen":{"limit":{"context":128000}}}},
        "alibaba-cn":{"models":{"qwen":{"limit":{"context":256000}}}}});
    std::fs::write(&path, catalog.to_string()).unwrap();
    let mut model = ModelIdentity {
        provider_id: "qwen".into(),
        model_id: "qwen".into(),
        revision: None,
    };
    assert!(crate::ai::model_metadata::cached_exact_context_window(&path, &model).is_none());
    let catalog = json!({"alibaba":{"models":{"qwen":{"limit":{"context":128000}}}}});
    std::fs::write(&path, catalog.to_string()).unwrap();
    assert_eq!(
        crate::ai::model_metadata::cached_exact_context_window(&path, &model)
            .unwrap()
            .tokens,
        128000
    );
    model.revision = Some("unverified".into());
    assert!(crate::ai::model_metadata::cached_exact_context_window(&path, &model).is_none());
    model.revision = None;
    model.provider_id = "openai".into();
    assert!(crate::ai::model_metadata::cached_exact_context_window(&path, &model).is_none());
}

struct CatalogAuth(std::sync::Mutex<Vec<tokio::sync::watch::Sender<uuid::Uuid>>>);
impl crate::agent::authentication::ChatGptCredentialSource for CatalogAuth {
    fn acquire(&self) -> crate::agent::authentication::CredentialFuture<'_> {
        Box::pin(async move {
            let (lease, sender) = crate::agent::authentication::tests::synthetic_lease_for_account(
                "catalog-preparation-fixture",
            );
            self.0.lock().unwrap().push(sender);
            Ok(lease)
        })
    }
}

#[tokio::test]
async fn catalog_authentication_failure_keeps_the_native_credential_error_class() {
    let mut f = fixture();
    let source = Arc::new(UnavailableChatGpt(std::sync::atomic::AtomicUsize::new(0)));
    f.resolver.chatgpt = Some(source.clone());
    f.resolver.codex_catalog = Some(Arc::new(
        crate::ai::codex_model_catalog::CodexModelCatalog::cached_fixture(
            source,
            f._directory.path().join("cache"),
            f._directory.path().join("overrides.json"),
        ),
    ));
    assert!(matches!(
        f.resolver
            .prepare(AgentHost::Writing, selection("openai", "gpt-5.6-terra"))
            .await,
        Err(AgentModelPreparationError::Credentials)
    ));
}

#[tokio::test]
async fn account_catalog_prepares_new_ids_and_restores_the_admitted_budget_after_refresh() {
    use crate::agent::{
        execution_profile::{AgentExecutionMode, execution_profile},
        repository::context_policy::FrozenContextPolicy,
    };
    use research_agent_core::submission::{SubmissionInput, SubmissionRecord};
    use uuid::Uuid;
    let mut f = fixture();
    let source = Arc::new(CatalogAuth(std::sync::Mutex::new(vec![])));
    let lease = source.acquire().await.unwrap();
    let directory = f._directory.path().join("codex-cache");
    std::fs::create_dir_all(&directory).unwrap();
    let cache_path = directory.join(format!("{}.json", lease.scope_id()));
    let catalog = |window, percent, trigger, truncation| {
        json!({
            "schema_version":2,"client_version":crate::ai::codex_model_catalog::CLIENT_VERSION,
            "fetched_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),"etag":null,
            "models":[{"slug":"new-account-model","display_name":"Account model","visibility":"list","context_window":window,
                "max_context_window":1050000,"effective_context_window_percent":percent,"auto_compact_token_limit":trigger,
                "truncation_policy":truncation,
                "input_modalities":["text","image"],"supported_reasoning_levels":[{"effort":"medium"}]}]
        })
    };
    let original_policy = json!({"mode":"tokens","limit":10000});
    let new_policy = json!({"mode":"bytes","limit":20000});
    std::fs::write(
        &cache_path,
        catalog(272000, 95, 240000, original_policy.clone()).to_string(),
    )
    .unwrap();
    f.resolver.chatgpt = Some(source.clone());
    f.resolver.codex_catalog = Some(Arc::new(
        crate::ai::codex_model_catalog::CodexModelCatalog::cached_fixture(
            source,
            directory,
            f._directory.path().join("overrides.json"),
        ),
    ));
    let prepared = f
        .resolver
        .prepare(AgentHost::Writing, selection("openai", "new-account-model"))
        .await
        .unwrap();
    assert_eq!(prepared.context_window_tokens(), Some(272000));
    assert_eq!(prepared.catalog_budget(), Some((95, Some(240000))));
    assert_eq!(
        serde_json::to_value(prepared.catalog_truncation_policy()).unwrap(),
        original_policy
    );
    let frozen = prepared.context_window_tokens();
    std::fs::write(
        &cache_path,
        catalog(500000, 90, 400000, new_policy.clone()).to_string(),
    )
    .unwrap();
    assert_eq!(prepared.context_window_tokens(), frozen);
    let latest = f
        .resolver
        .prepare(AgentHost::Writing, selection("openai", "new-account-model"))
        .await
        .unwrap();
    assert_eq!(latest.context_window_tokens(), Some(500000));
    assert_eq!(latest.catalog_budget(), Some((90, Some(400000))));
    assert_eq!(
        serde_json::to_value(latest.catalog_truncation_policy()).unwrap(),
        new_policy
    );
    assert!(
        f.resolver
            .prepare(AgentHost::Writing, selection("openai", "invented-model"))
            .await
            .is_err()
    );

    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let store = crate::storage::AppStore::for_test(db, f._directory.path().join("objects"));
    let record = SubmissionRecord {
        conversation_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        input: SubmissionInput {
            remote_compaction_v2: None,
            model: prepared.identity().clone(),
            reasoning_effort: None,
            service_tier: None,
            text: "fixture".into(),
            context: vec![],
            user_input: vec![],
            skill_instructions: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        },
    };
    {
        let connection = store.shared_agent_connection();
        let db = connection.lock().unwrap();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'catalog fixture')",
            [record.conversation_id.to_string()],
        )
        .unwrap();
        db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,?5)",rusqlite::params![record.conversation_id.to_string(),record.request_id.to_string(),record.turn_id.to_string(),record.context_snapshot_id.to_string(),serde_json::to_string(&record.input).unwrap()]).unwrap();
    }
    let profile =
        execution_profile(&record.input.model, AgentExecutionMode::ChatGptDefault).unwrap();
    let transport = profile.transport(&store, &record, prepared).unwrap();
    assert_eq!(
        transport.tool_output_policy(),
        Some(research_agent_core::model::tool_output::ToolOutputPolicy::Tokens(10000))
    );
    assert_eq!(transport.capabilities().reasoning_efforts, vec!["medium"]);
    assert!(
        !transport.capabilities().vision,
        "writing host remains text-only"
    );
    transport.prepare_context_policy().await.unwrap();
    let reopened = profile.transport(&store, &record, latest).unwrap();
    assert_eq!(
        reopened.tool_output_policy(),
        Some(research_agent_core::model::tool_output::ToolOutputPolicy::Tokens(10000))
    );
    reopened.prepare_context_policy().await.unwrap();
    let connection = store.shared_agent_connection();
    let encoded: String = connection
        .lock()
        .unwrap()
        .query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let policy: FrozenContextPolicy = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        serde_json::to_value(policy.tool_truncation_policy).unwrap(),
        original_policy
    );
    let budget = policy.budget.unwrap();
    assert_eq!(budget.context_window_tokens, 272000);
    assert_eq!(budget.effective_percent, 95);
    assert_eq!(budget.configured_trigger_tokens, Some(240000));

    // Old persisted policies have no truncation field: a newer catalog must
    // not retrofit metadata into the identity of an already admitted turn.
    let mut old: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    old.as_object_mut()
        .unwrap()
        .remove("tool_truncation_policy");
    let mut old_record = record.clone();
    old_record.request_id = Uuid::new_v4();
    old_record.turn_id = Uuid::new_v4();
    old_record.context_snapshot_id = Uuid::new_v4();
    {
        let db = connection.lock().unwrap();
        db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,?5)", rusqlite::params![old_record.conversation_id.to_string(), old_record.request_id.to_string(), old_record.turn_id.to_string(), old_record.context_snapshot_id.to_string(), serde_json::to_string(&old_record.input).unwrap()]).unwrap();
        db.execute(
            "insert into agent_context_policies(turn_id,policy_json) values(?1,?2)",
            rusqlite::params![old_record.turn_id.to_string(), old.to_string()],
        )
        .unwrap();
    }
    let latest = f
        .resolver
        .prepare(AgentHost::Writing, selection("openai", "new-account-model"))
        .await
        .unwrap();
    let old_transport = profile.transport(&store, &old_record, latest).unwrap();
    assert_eq!(old_transport.tool_output_policy(), None);
    old_transport.prepare_context_policy().await.unwrap();
    let saved = crate::agent::repository::context_policy::SqliteContextPolicy::restored(
        &connection,
        &old_record,
    )
    .unwrap()
    .unwrap();
    assert_eq!(saved.tool_truncation_policy, None);
}
