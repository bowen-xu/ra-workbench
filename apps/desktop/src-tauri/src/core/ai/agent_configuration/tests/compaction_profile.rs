//! Real native composition with synthetic credential sources; no inference.
use super::*;
use crate::agent::execution_profile::{AgentExecutionMode, execution_profile_for_host};
use research_agent_core::{
    context::compaction_strategy::{CompactionKind, CompactionRetention},
    submission::{SubmissionInput, SubmissionRecord},
};

#[tokio::test]
async fn host_profiles_freeze_v2_for_chatgpt_and_text_for_chat_without_auth_io() {
    let mut fixture = fixture();
    let source = Arc::new(UnavailableChatGpt(std::sync::atomic::AtomicUsize::new(0)));
    fixture.resolver.chatgpt = Some(source.clone());
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let store = crate::storage::AppStore::for_test(db, fixture._directory.path().join("objects"));
    for host in [AgentHost::Writing, AgentHost::Notes, AgentHost::Reading] {
        for (provider, model, mode, kind) in [
            (
                "openai",
                "gpt-5.6-terra",
                AgentExecutionMode::ChatGptDefault,
                CompactionKind::RemoteOpaque,
            ),
            (
                "deepseek",
                "deepseek-v4-pro",
                AgentExecutionMode::DeepSeekNonThinking,
                CompactionKind::TextSummary,
            ),
        ] {
            let prepared = fixture
                .resolver
                .prepare(host, selection(provider, model))
                .await
                .unwrap();
            let identity = prepared.identity().clone();
            let mut profile = execution_profile_for_host(&identity, mode, host).unwrap();
            if kind == CompactionKind::TextSummary {
                profile = profile
                    .with_compact_prompt(&identity, Some("frozen model summary policy".into()))
                    .unwrap();
            } else {
                assert!(
                    profile
                        .clone()
                        .with_compact_prompt(&identity, Some("must not reach opaque path".into()))
                        .is_err()
                );
            }
            let record = SubmissionRecord {
                conversation_id: uuid::Uuid::new_v4(),
                request_id: uuid::Uuid::new_v4(),
                turn_id: uuid::Uuid::new_v4(),
                context_snapshot_id: uuid::Uuid::new_v4(),
                input: SubmissionInput {
                    remote_compaction_v2: None,
                    model: identity.clone(),
                    reasoning_effort: prepared.reasoning_effort().map(str::to_owned),
                    service_tier: prepared.service_tier().map(str::to_owned),
                    text: "/compact".into(),
                    context: vec![],
                    user_input: vec![],
                    skill_instructions: vec![],
                    instructions: vec![],
                    instruction_configs: vec![],
                    skills: vec![],
                },
            };
            let transport = profile.transport(&store, &record, prepared).unwrap();
            if kind == CompactionKind::TextSummary {
                profile = profile
                    .with_compact_prompt(
                        &identity,
                        Some("changed after transport composition".into()),
                    )
                    .unwrap();
            }
            let strategy = profile.compaction_strategy(transport.clone()).unwrap();
            if kind == CompactionKind::TextSummary {
                assert_eq!(
                    strategy.text_compaction_prompt(),
                    Some("frozen model summary policy")
                );
            }
            assert_eq!(strategy.binding().model, identity);
            assert_eq!(strategy.binding().kind, kind);
            let prepared_strategy = transport.prepared_compaction_strategy().unwrap();
            assert!(prepared_strategy.binding() == strategy.binding());
            if kind == CompactionKind::RemoteOpaque {
                assert!(Arc::ptr_eq(&strategy, &prepared_strategy));
            }
            // Text compaction is recomposed around the host/accounting transport;
            // its strategy allocation need not be the inner wire strategy.
            assert!(transport.supports_compaction_kind(kind));
            assert!(
                strategy.retention_policy()
                    == Some(if kind == CompactionKind::RemoteOpaque {
                        CompactionRetention::RecentUserMessages
                    } else {
                        CompactionRetention::RecentUserText
                    })
            );
            // The same real native factories can opt into reset without asking
            // the provider for an opaque state or summary capability.
            assert!(transport.token_budget_policy().is_none());
            let settings = research_agent_core::context::token_budget::TokenBudgetSettings {
                reminder_threshold_tokens: Some(1234),
                guidance_message: Some(" preserve anchors ".into()),
                auto_compact_fallback_prompt: Some("  finish current tool  ".into()),
                auto_compact_fallback_buffer_tokens: Some(200),
                ..Default::default()
            };
            let reset_profile = profile
                .with_token_budget_settings(&identity, settings)
                .unwrap();

            let prepared = fixture
                .resolver
                .prepare(host, selection(provider, model))
                .await
                .unwrap();
            let reset_transport = reset_profile.transport(&store, &record, prepared).unwrap();
            let runtime_policy = reset_transport.token_budget_policy().unwrap();
            assert_eq!(
                runtime_policy.settings().reminder_threshold_tokens,
                Some(1234)
            );
            assert_eq!(
                runtime_policy.settings().guidance_message.as_deref(),
                Some(" preserve anchors ")
            );
            assert_eq!(
                runtime_policy
                    .settings()
                    .auto_compact_fallback_prompt
                    .as_deref(),
                Some("finish current tool")
            );
            assert_eq!(runtime_policy.fallback_buffer_tokens(), 200);
            let changed_profile = reset_profile
                .clone()
                .with_token_budget_settings(
                    &identity,
                    research_agent_core::context::token_budget::TokenBudgetSettings {
                        reminder_threshold_tokens: Some(4321),
                        ..Default::default()
                    },
                )
                .unwrap();
            let reset = reset_profile
                .compaction_strategy(reset_transport.clone())
                .unwrap();
            assert_eq!(reset.binding().kind, CompactionKind::ResetWindow);
            assert_eq!(reset.binding().model, identity);
            assert!(reset.retention_policy() == Some(CompactionRetention::ResetWindow));
            assert!(reset.text_compaction_prompt().is_none());
            assert!(
                reset_transport
                    .pre_turn_compaction(uuid::Uuid::new_v4(), &identity, Some(u64::MAX))
                    .await
                    .unwrap()
                    .is_none()
            );
            {
                let connection = store.shared_agent_connection();
                let db = connection.lock().unwrap();
                db.execute(
                    "insert into agent_conversations(id,title) values(?1,'reset fixture')",
                    [record.conversation_id.to_string()],
                )
                .unwrap();
                db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,?5)", rusqlite::params![record.conversation_id.to_string(),record.request_id.to_string(),record.turn_id.to_string(),record.context_snapshot_id.to_string(),serde_json::to_string(&record.input).unwrap()]).unwrap();
            }
            reset_transport.prepare_context_policy().await.unwrap();
            reset_transport.prepare_context_policy().await.unwrap();
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
            let frozen: crate::agent::repository::context_policy::FrozenContextPolicy =
                serde_json::from_str(&encoded).unwrap();
            frozen.validate().unwrap();
            assert_eq!(frozen.binding, reset.binding());
            assert!(frozen.compact_prompt.is_none());
            assert!(
                frozen.token_budget_policy().unwrap().unwrap().settings()
                    == runtime_policy.settings()
            );
            let prepared = fixture
                .resolver
                .prepare(host, selection(provider, model))
                .await
                .unwrap();
            let changed_transport = changed_profile
                .transport(&store, &record, prepared)
                .unwrap();
            assert!(matches!(
                changed_transport.prepare_context_policy().await,
                Err(research_agent_core::model::ModelError::InvalidRequest)
            ));
            assert_eq!(
                reset_transport
                    .token_budget_policy()
                    .unwrap()
                    .settings()
                    .reminder_threshold_tokens,
                Some(1234)
            );

            // Changing a host's choice after admission cannot overwrite this
            // turn's frozen reset policy with its former summary policy.
            assert!(matches!(
                transport.prepare_context_policy().await,
                Err(research_agent_core::model::ModelError::InvalidRequest)
            ));
        }
    }
    assert_eq!(source.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}
