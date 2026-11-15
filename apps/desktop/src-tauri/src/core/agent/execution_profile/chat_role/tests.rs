use super::*;
use crate::agent::model::{AgentHost, AgentModelResolver, AgentModelSelection};
use crate::agent::repository::{SqliteSubmissionStore, context_policy::*, create_conversation};
use research_agent_core::{
    context::{checkpoint::*, compaction_strategy::*},
    conversation::ConversationId,
    model::*,
    submission::*,
};

#[tokio::test]
async fn native_factory_freezes_role_and_restores_it_without_allowing_same_turn_drift() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("role.sqlite");
    let mut db = rusqlite::Connection::open(&database).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = uuid::Uuid::new_v4();
    create_conversation(&mut db, ConversationId::from_uuid(id), "Role fixture").unwrap();
    db.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values ('deepseek',1,'',60000,'fixture') on conflict(provider_id) do update set enabled=1,base_url=excluded.base_url", []).unwrap();
    let app = AppStore::for_test(db, root.path().join("objects"));
    let credentials = crate::ai::JsonCredentialStore::from_path(root.path().join("synthetic.json"));
    credentials
        .set("deepseek", "synthetic-role-fixture")
        .unwrap();
    let resolver = AgentModelResolver::from_app_store(&app, credentials);
    let connection = app.shared_agent_connection();
    let old_connection = Arc::downgrade(&connection);
    drop(connection);
    let model = ModelIdentity {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        revision: None,
    };
    let prepare = || {
        resolver.prepare(
            AgentHost::Writing,
            Some(AgentModelSelection {
                provider_id: model.provider_id.clone(),
                model_id: model.model_id.clone(),
                reasoning_effort: None,
                service_tier: None,
            }),
        )
    };
    let store = SqliteSubmissionStore::new(app.shared_agent_connection());
    let record = store
        .put(
            id,
            uuid::Uuid::new_v4(),
            SubmissionInput {
                remote_compaction_v2: None,
                user_input: vec![],
                skill_instructions: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
                text: "input".into(),
                model: model.clone(),
                reasoning_effort: None,
                service_tier: None,
                context: vec![],
            },
        )
        .await
        .unwrap();
    let base = execution_profile(&model, AgentExecutionMode::DeepSeekNonThinking)
        .unwrap()
        .with_token_budget_reset(&model)
        .unwrap();
    let mut other = model.clone();
    other.revision = Some("unreviewed".into());
    assert!(base.clone().with_chat_developer_role(&other, true).is_err());
    let declared = base.clone().with_chat_developer_role(&model, true).unwrap();
    let transport = declared
        .transport(&app, &record, prepare().await.unwrap())
        .unwrap();
    transport.prepare_context_policy().await.unwrap();
    let saved: String = app
        .connection()
        .unwrap()
        .query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let snapshot: FrozenContextPolicy = serde_json::from_str(&saved).unwrap();
    assert_eq!(snapshot.chat_developer_model, Some(model.clone()));
    let request = ModelRequest {
        captured_history: None,
        request_id: uuid::Uuid::new_v4(),
        model: model.clone(),
        messages: vec![ModelMessage::Developer("trusted restored guidance".into())],
        tools: vec![],
        max_output_tokens: declared.max_output_tokens,
        parallel_tool_calls: false,
        reasoning_effort: None,
    };
    let window = ContextCheckpoint {
        schema_version: 3,
        window_id: uuid::Uuid::new_v4(),
        conversation_id: id,
        turn_id: record.turn_id,
        source_watermark: 0,
        previous_window_id: None,
        binding: snapshot.binding.clone(),
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::ResetWindow,
        retained: vec![],
        canonical_context: Some(CanonicalContext {
            before_retained: 0,
            messages: store_messages(&request.messages).unwrap(),
        }),
    };
    transport
        .validate_compaction_window(&window, &request)
        .await
        .unwrap();
    drop(transport);
    let restored = declared
        .transport_inner(
            &app,
            &record,
            prepare().await.unwrap(),
            None,
            Some(snapshot.clone()),
        )
        .unwrap();
    restored.prepare_context_policy().await.unwrap();
    restored
        .validate_compaction_window(&window, &request)
        .await
        .unwrap();
    // A fresh declaration cannot alter a policy that has already been frozen.
    let drift = base
        .transport(&app, &record, prepare().await.unwrap())
        .unwrap();
    assert_eq!(
        drift.prepare_context_policy().await,
        Err(ModelError::InvalidRequest)
    );
    assert!(
        drift
            .validate_compaction_window(&window, &request)
            .await
            .is_err()
    );
    assert!(
        base.transport_inner(
            &app,
            &record,
            prepare().await.unwrap(),
            None,
            Some(snapshot.clone())
        )
        .is_err()
    );
    let unchanged: String = app
        .connection()
        .unwrap()
        .query_row(
            "select policy_json from agent_context_policies where turn_id=?1",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, unchanged);
    let mut legacy = serde_json::to_value(&snapshot).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("chat_developer_model");
    let legacy: FrozenContextPolicy = serde_json::from_value(legacy).unwrap();
    assert!(legacy.chat_developer_model.is_none());
    legacy.validate().unwrap();
    let mut invalid = snapshot.clone();
    invalid.chat_developer_model = Some(other);
    assert_eq!(invalid.validate(), Err(ModelError::InvalidRequest));
    let mut invalid = snapshot;
    invalid.authentication = ContextAuthentication::CodexBackend;
    assert_eq!(invalid.validate(), Err(ModelError::InvalidRequest));

    // Close every owner before reopening: a live connection cannot stand in
    // for restoration from the persisted policy after an application restart.
    drop(restored);
    drop(drift);
    drop(store);
    drop(resolver);
    drop(app);
    assert!(old_connection.upgrade().is_none());
    let app = AppStore::for_test(
        rusqlite::Connection::open(database).unwrap(),
        root.path().join("objects"),
    );
    let credentials = crate::ai::JsonCredentialStore::from_path(root.path().join("synthetic.json"));
    let resolver = AgentModelResolver::from_app_store(&app, credentials);
    let frozen: FrozenContextPolicy = serde_json::from_str(
        &app.connection()
            .unwrap()
            .query_row(
                "select policy_json from agent_context_policies where turn_id=?1",
                [record.turn_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(frozen.chat_developer_model, Some(model.clone()));
    let declared = execution_profile(&model, AgentExecutionMode::DeepSeekNonThinking)
        .unwrap()
        .with_token_budget_reset(&model)
        .unwrap()
        .with_chat_developer_role(frozen.chat_developer_model.as_ref().unwrap(), true)
        .unwrap();
    let prepared = resolver
        .prepare(
            AgentHost::Writing,
            Some(AgentModelSelection {
                provider_id: model.provider_id.clone(),
                model_id: model.model_id.clone(),
                reasoning_effort: None,
                service_tier: None,
            }),
        )
        .await
        .unwrap();
    let restored = declared
        .transport_inner(&app, &record, prepared, None, Some(frozen))
        .unwrap();
    restored.prepare_context_policy().await.unwrap();
    restored
        .validate_compaction_window(&window, &request)
        .await
        .unwrap();
}
