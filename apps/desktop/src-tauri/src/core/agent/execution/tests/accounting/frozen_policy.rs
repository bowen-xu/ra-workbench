use super::*;
use crate::agent::repository::context_policy::{
    ContextAuthentication, FrozenContextPolicy, SqliteContextPolicy,
};
use research_agent_core::context::compaction_strategy::{CompactionBinding, CompactionKind};

fn policy() -> FrozenContextPolicy {
    FrozenContextPolicy {
        tool_truncation_policy: None,
        reasoning_summary: None,
        chat_developer_model: None,
        compact_prompt: None,
        token_budget_settings: None,
        schema_version: 1,
        binding: CompactionBinding {
            model: model(),
            strategy_id: "research.text.v1".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        budget: Some(CompactionBudget {
            context_window_tokens: 10000,
            effective_percent: 95,
            configured_trigger_tokens: Some(5000),
            output_reserve_tokens: 100,
        }),
        max_output_tokens: 100,
        scope: research_agent_core::context::accounting::AutoCompactionScope::Total,
        unknown_window_trigger_tokens: Some(5000),
        comp_hash: Some("trusted-codec-v1".into()),
        authentication: ContextAuthentication::ApiKey,
    }
}

#[tokio::test]
async fn reasoning_summary_policy_restores_and_rejects_retry_drift_without_rewriting_history() {
    use crate::agent::model::responses_reasoning::ResponsesReasoningSummary;
    let ((_, store, _, _, _, id), db) = setup_with_connection().await;
    let record = store
        .put(id, Uuid::new_v4(), input("frozen reasoning"))
        .await
        .unwrap();
    let mut saved = policy();
    saved.authentication = ContextAuthentication::CodexBackend;
    saved.reasoning_summary = Some(ResponsesReasoningSummary::Detailed);
    SqliteContextPolicy::new(db.clone(), &record, saved.clone())
        .unwrap()
        .prepare()
        .await
        .unwrap();
    let encoded: String = db
        .lock()
        .unwrap()
        .query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let restored: FrozenContextPolicy = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.reasoning_summary, saved.reasoning_summary);
    SqliteContextPolicy::new(db.clone(), &record, restored.clone())
        .unwrap()
        .prepare()
        .await
        .unwrap();
    let mut drift = restored;
    drift.reasoning_summary = Some(ResponsesReasoningSummary::Concise);
    assert_eq!(
        SqliteContextPolicy::new(db.clone(), &record, drift)
            .unwrap()
            .prepare()
            .await,
        Err(ModelError::InvalidRequest)
    );
    let unchanged: String = db
        .lock()
        .unwrap()
        .query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unchanged, encoded);
    let mut legacy = serde_json::to_value(&saved).unwrap();
    legacy.as_object_mut().unwrap().remove("reasoning_summary");
    assert_eq!(
        serde_json::from_value::<FrozenContextPolicy>(legacy.clone())
            .unwrap()
            .reasoning_summary,
        None
    );
    legacy["reasoning_summary"] = serde_json::json!("unsupported");
    assert!(serde_json::from_value::<FrozenContextPolicy>(legacy).is_err());
    saved.authentication = ContextAuthentication::ApiKey;
    assert_eq!(saved.validate(), Err(ModelError::InvalidRequest));
}

#[test]
fn unknown_window_snapshot_preserves_unknowns_and_explicit_output_reserve() {
    let mut policy = policy();
    policy.budget = None;
    policy.comp_hash = None;
    policy.scope = research_agent_core::context::accounting::AutoCompactionScope::BodyAfterPrefix;
    let encoded = serde_json::to_string(&policy).unwrap();
    let decoded: FrozenContextPolicy = serde_json::from_str(&encoded).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, policy);
    assert_eq!(decoded.max_output_tokens, 100);
    let mut invalid = policy.clone();
    invalid.max_output_tokens = 0;
    assert_eq!(invalid.validate(), Err(ModelError::InvalidRequest));
    let mut value = serde_json::to_value(&policy).unwrap();
    value["credential"] = serde_json::json!("forbidden-field");
    assert!(serde_json::from_value::<FrozenContextPolicy>(value).is_err());
}
struct PolicyTransport {
    inner: Arc<Transport>,
    policy: SqliteContextPolicy,
}
impl ModelTransport for PolicyTransport {
    fn prepare_context_policy(&self) -> ModelFuture<'_, ()> {
        self.policy.prepare()
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}

#[tokio::test]
async fn native_preparation_freezes_policy_before_inference_and_rejects_drift() {
    let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let record = store
        .put(id, Uuid::new_v4(), input("policy"))
        .await
        .unwrap();
    let mut saved_policy = policy();
    saved_policy.compact_prompt = Some("保留\n证据锚点".repeat(1000));
    execution.transport = Arc::new(PolicyTransport {
        inner: transport.clone(),
        policy: SqliteContextPolicy::new(db.clone(), &record, saved_policy.clone()).unwrap(),
    });
    assert_eq!(
        run(Arc::new(execution), service, store, id, record.request_id).await,
        TaskCompletion::Completed
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    let encoded: String = db
        .lock()
        .unwrap()
        .query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<FrozenContextPolicy>(&encoded).unwrap(),
        saved_policy
    );
    SqliteContextPolicy::new(db.clone(), &record, saved_policy.clone())
        .unwrap()
        .prepare()
        .await
        .unwrap();
    let mut changed_prompt = saved_policy.clone();
    changed_prompt.compact_prompt = Some("changed summary policy".into());
    assert_eq!(
        SqliteContextPolicy::new(db.clone(), &record, changed_prompt)
            .unwrap()
            .prepare()
            .await,
        Err(ModelError::InvalidRequest)
    );
    let mut changed = saved_policy.clone();
    changed.budget.as_mut().unwrap().context_window_tokens = 20000;
    assert_eq!(
        SqliteContextPolicy::new(db.clone(), &record, changed)
            .unwrap()
            .prepare()
            .await,
        Err(ModelError::InvalidRequest)
    );
    let mut changed = saved_policy;
    changed.authentication = ContextAuthentication::CodexBackend;
    assert_eq!(
        SqliteContextPolicy::new(db.clone(), &record, changed)
            .unwrap()
            .prepare()
            .await,
        Err(ModelError::InvalidRequest)
    );
    let db = db.lock().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let disk = directory.path().join("policy.sqlite");
    db.execute("vacuum into ?1", [disk.to_str().unwrap()])
        .unwrap();
    let reopened = rusqlite::Connection::open(&disk).unwrap();
    let saved: String = reopened
        .query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, encoded);
    assert!(
        db.execute(
            "update agent_context_policies set policy_json=policy_json where turn_id=?1",
            [record.turn_id.to_string()]
        )
        .is_err()
    );
    db.execute(
        "delete from agent_conversations where id=?1",
        [id.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row::<u64, _, _>("select count(*) from agent_context_policies", [], |row| row
            .get(0))
            .unwrap(),
        0
    );
}

#[test]
fn migration_upgrades_existing_databases_without_fabricating_old_policy() {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 129);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        db.query_row::<u64, _, _>("select count(*) from agent_context_policies", [], |row| row
            .get(0))
            .unwrap(),
        0
    );
    assert!(db.execute("insert into agent_context_policies(turn_id,policy_json) values('missing-submission','{}')", []).is_err());
}

#[tokio::test]
async fn invalid_saved_policy_fails_before_model_or_tools() {
    let ((service, store, transport, tools, mut execution, id), db) = setup_with_connection().await;
    let record = store
        .put(id, Uuid::new_v4(), input("policy"))
        .await
        .unwrap();
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_context_policies(turn_id,policy_json) values(?1,'{}')",
            [record.turn_id.to_string()],
        )
        .unwrap();
    execution.transport = Arc::new(PolicyTransport {
        inner: transport.clone(),
        policy: SqliteContextPolicy::new(db.clone(), &record, policy()).unwrap(),
    });
    assert_eq!(
        run(Arc::new(execution), service, store, id, record.request_id).await,
        TaskCompletion::Failed
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn compact_prompt_migration_preserves_legacy_policy_and_immutability() {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 131);
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut connection,
        ConversationId::from_uuid(id),
        "policy",
    )
    .unwrap();
    let db = Arc::new(Mutex::new(connection));
    let submissions = SqliteSubmissionStore::new(db.clone());
    let record = submissions
        .put(id, Uuid::new_v4(), input("legacy-policy"))
        .await
        .unwrap();
    SqliteContextPolicy::new(db.clone(), &record, policy())
        .unwrap()
        .prepare()
        .await
        .unwrap();
    let read = |db: &Connection| -> String {
        db.query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap()
    };
    let mut db = db.lock().unwrap();
    let before = read(&db);
    assert!(!before.contains("compact_prompt"));
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(read(&db), before);
    assert_eq!(
        serde_json::from_str::<FrozenContextPolicy>(&before)
            .unwrap()
            .compact_prompt,
        None
    );
    assert!(
        db.execute(
            "update agent_context_policies set policy_json=policy_json",
            []
        )
        .is_err()
    );
    db.execute(
        "delete from agent_conversations where id=?1",
        [id.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_context_policies", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn token_budget_snapshots_validate_normalize_and_keep_legacy_summary_disabled() {
    use research_agent_core::context::token_budget::TokenBudgetSettings;
    let mut saved = policy();
    let legacy = serde_json::to_value(&saved).unwrap();
    assert!(legacy.get("token_budget_settings").is_none());
    let legacy: FrozenContextPolicy = serde_json::from_value(legacy).unwrap();
    assert!(legacy.token_budget_policy().unwrap().is_none());
    saved.token_budget_settings = Some(TokenBudgetSettings::default().try_into().unwrap());
    assert_eq!(saved.validate(), Err(ModelError::InvalidRequest));
    saved.binding =
        crate::agent::execution_profile::NativeCompactionPolicy::TokenBudgetReset.binding(model());
    saved.token_budget_settings = None;
    saved.validate().unwrap();
    assert!(
        saved.token_budget_policy().unwrap().unwrap().settings() == &TokenBudgetSettings::default()
    );
    saved.token_budget_settings = Some(
        TokenBudgetSettings {
            reminder_threshold_tokens: Some(200),
            guidance_message: Some(" \n ".into()),
            auto_compact_fallback_prompt: Some("  fallback  ".into()),
            auto_compact_fallback_buffer_tokens: Some(30),
            ..Default::default()
        }
        .try_into()
        .unwrap(),
    );
    let encoded = serde_json::to_value(&saved).unwrap();
    let restored: FrozenContextPolicy = serde_json::from_value(encoded.clone()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, saved);
    let resolved = restored.token_budget_policy().unwrap().unwrap();
    assert!(resolved.settings().guidance_message.is_none());
    assert_eq!(
        resolved.settings().auto_compact_fallback_prompt.as_deref(),
        Some("fallback")
    );
    assert!(!format!("{:?}", restored.token_budget_settings).contains("fallback\""));
    for (key, value) in [
        ("reminder_threshold_tokens", serde_json::json!(0)),
        (
            "auto_compact_fallback_buffer_tokens",
            serde_json::Value::Null,
        ),
        ("reminder_message_template", serde_json::json!(" ")),
        ("guidance_message", serde_json::json!("x".repeat(2001))),
        ("unknown_setting", serde_json::json!(true)),
    ] {
        let mut invalid = encoded.clone();
        invalid["token_budget_settings"][key] = value;
        assert!(
            serde_json::from_value::<FrozenContextPolicy>(invalid).is_err(),
            "{key}"
        );
    }
    saved.compact_prompt = Some("summary only".into());
    assert_eq!(saved.validate(), Err(ModelError::InvalidRequest));
}
