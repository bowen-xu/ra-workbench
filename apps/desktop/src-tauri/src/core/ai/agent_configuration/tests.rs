use super::*;
use crate::data_interface::IntoDatabase;
use rusqlite::params;
#[path = "tests/compaction_profile.rs"]
mod compaction_profile;
#[path = "tests/context_metadata.rs"]
mod context_metadata;

struct Fixture {
    resolver: AgentModelResolver,
    _directory: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let credentials =
        JsonCredentialStore::from_path(directory.path().join("fixture-credentials.json"));
    credentials.set("deepseek", "synthetic-deepseek").unwrap();
    credentials.set("qwen", "synthetic-qwen").unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 67);
    for provider in ["deepseek", "qwen", "openai"] {
        connection.execute("insert into llm_provider_configs (provider_id,enabled,base_url,timeout_ms,updated_at) values (?1,1,'',60000,'fixture') on conflict(provider_id) do update set enabled=1,base_url=''",[provider]).unwrap();
    }
    let fixture = Fixture {
        resolver: AgentModelResolver {
            #[cfg(all(debug_assertions, feature = "development-data"))]
            preview: None,
            #[cfg(target_os = "macos")]
            native_hooks: None,
            compaction_hooks: None,
            connection: Arc::new(Mutex::new(connection)).into_database(),
            credentials,
            chatgpt: None,
            context_metadata: None,
            codex_catalog: None,
        },
        _directory: directory,
    };
    route(&fixture, "default", "text", "deepseek", "global-model");
    fixture
}
fn route(fixture: &Fixture, kind: &str, id: &str, provider: &str, model: &str) {
    fixture.resolver.connection.lock().unwrap().execute("insert into llm_model_routes (route_kind,route_id,model_kind,provider_id,model_id,updated_at) values (?1,?2,'text',?3,?4,'fixture') on conflict(route_kind,route_id) do update set provider_id=excluded.provider_id,model_id=excluded.model_id",params![kind,id,provider,model]).unwrap();
}
fn selection(provider: &str, model: &str) -> Option<AgentModelSelection> {
    Some(AgentModelSelection {
        provider_id: provider.into(),
        model_id: model.into(),
        reasoning_effort: None,
        service_tier: None,
    })
}

#[tokio::test]
async fn override_then_host_route_then_default_and_snapshot_remains_frozen() {
    let fixture = fixture();
    route(
        &fixture,
        "use_case",
        "paper_reading_chat",
        "qwen",
        "reading-model",
    );
    let reading = fixture
        .resolver
        .prepare(AgentHost::Reading, None)
        .await
        .unwrap();
    let writing = fixture
        .resolver
        .prepare(AgentHost::Writing, None)
        .await
        .unwrap();
    let override_model = fixture
        .resolver
        .prepare(AgentHost::Reading, selection("deepseek", " once "))
        .await
        .unwrap();
    assert_eq!(reading.identity().model_id, "reading-model");
    assert_eq!(writing.identity().model_id, "global-model");
    assert_eq!(override_model.identity().model_id, "once");
    route(&fixture, "default", "text", "deepseek", "new-default");
    assert_eq!(writing.identity().model_id, "global-model");
    assert_eq!(
        fixture
            .resolver
            .prepare(AgentHost::Notes, None)
            .await
            .unwrap()
            .identity()
            .model_id,
        "new-default"
    );
    assert_eq!(
        fixture
            .resolver
            .prepare(AgentHost::Reading, None)
            .await
            .unwrap()
            .identity()
            .model_id,
        "reading-model"
    );
    // This creates only an HTTP client, not a provider request.
    writing
        .into_chat_transport(
            ModelCapabilities {
                vision: false,
                tools: true,
                parallel_tools: false,
                reasoning_efforts: vec![],
            },
            OutputTokenParameter::MaxTokens,
        )
        .unwrap();
}

#[tokio::test]
async fn disabled_missing_or_unknown_selection_never_falls_back_to_global() {
    let fixture = fixture();
    fixture
        .resolver
        .connection
        .lock()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=0 where provider_id='qwen'",
            [],
        )
        .unwrap();
    assert!(matches!(
        fixture
            .resolver
            .prepare(AgentHost::Reading, selection("qwen", "model"))
            .await,
        Err(AgentModelPreparationError::Disabled)
    ));
    assert!(matches!(
        fixture
            .resolver
            .prepare(AgentHost::Review, selection("unknown", "model"))
            .await,
        Err(AgentModelPreparationError::Settings)
    ));
    fixture.resolver.credentials.delete("deepseek").unwrap();
    assert!(matches!(
        fixture.resolver.prepare(AgentHost::Writing, None).await,
        Err(AgentModelPreparationError::Credentials)
    ));
}

#[tokio::test]
async fn chatgpt_selection_does_not_read_api_key_store_or_invoke_old_runtime() {
    let mut fixture = fixture();
    // A directory is an invalid credential file. ChatGPT selection must return
    // the dedicated transport error before any API-key file operation.
    fixture.resolver.credentials =
        JsonCredentialStore::from_path(fixture._directory.path().to_owned());
    assert!(matches!(
        fixture
            .resolver
            .prepare(AgentHost::Writing, selection("openai", "codex-model"))
            .await,
        Err(AgentModelPreparationError::ChatGptTransportUnavailable)
    ));
}

#[tokio::test]
async fn stored_endpoint_tampering_and_invalid_override_fail_before_transport() {
    let fixture = fixture();
    assert!(matches!(
        fixture
            .resolver
            .prepare(AgentHost::Writing, selection("deepseek", " "))
            .await,
        Err(AgentModelPreparationError::Settings)
    ));
    fixture.resolver.connection.lock().unwrap().execute("update llm_provider_configs set base_url='https://invalid.example' where provider_id='deepseek'",[]).unwrap();
    assert!(matches!(
        fixture.resolver.prepare(AgentHost::Writing, None).await,
        Err(AgentModelPreparationError::Settings)
    ));
}

#[tokio::test]
async fn deleted_routes_leave_no_implicit_provider_default() {
    let fixture = fixture();
    fixture
        .resolver
        .connection
        .lock()
        .unwrap()
        .execute("delete from llm_model_routes", [])
        .unwrap();
    assert!(matches!(
        fixture.resolver.prepare(AgentHost::Notes, None).await,
        Err(AgentModelPreparationError::Settings)
    ));
}

#[tokio::test]
async fn explicit_reasoning_survives_legacy_normalization_and_freezes_before_transport() {
    let fixture = fixture();
    let prepared = fixture
        .resolver
        .prepare(
            AgentHost::Writing,
            Some(AgentModelSelection {
                provider_id: "deepseek".into(),
                model_id: "fixture".into(),
                reasoning_effort: Some("medium".into()),
                service_tier: None,
            }),
        )
        .await
        .unwrap();
    assert_eq!(prepared.reasoning_effort(), Some("medium"));
    assert!(matches!(
        prepared.into_chat_transport(
            ModelCapabilities {
                vision: false,
                tools: true,
                parallel_tools: false,
                reasoning_efforts: vec![],
            },
            OutputTokenParameter::MaxTokens
        ),
        Err(AgentModelPreparationError::Transport(
            ModelError::UnsupportedCapability
        ))
    ));
}

struct UnavailableChatGpt(std::sync::atomic::AtomicUsize);
impl crate::agent::authentication::ChatGptCredentialSource for UnavailableChatGpt {
    fn acquire(&self) -> crate::agent::authentication::CredentialFuture<'_> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { Err(crate::agent::authentication::AuthenticationError::Unavailable) })
    }
}
#[tokio::test]
async fn cached_fixture_prepares_without_touching_keys_or_starting_inference() {
    use crate::agent::model::responses_continuation::ResponsesContinuation;
    use research_agent_core::model::{ContentPart, ModelMessage, ModelRequest, ModelTransport};
    let mut fixture = fixture();
    fixture.resolver.credentials =
        JsonCredentialStore::from_path(fixture._directory.path().to_owned());
    let source = Arc::new(UnavailableChatGpt(std::sync::atomic::AtomicUsize::new(0)));
    fixture.resolver.chatgpt = Some(source.clone());
    let prepared = fixture
        .resolver
        .prepare(AgentHost::Writing, selection("openai", "gpt-5.6-terra"))
        .await
        .unwrap();
    assert!(prepared.is_chatgpt());
    let model = prepared.identity().clone();
    let effort = prepared.reasoning_effort().map(str::to_owned);
    let state =
        Arc::new(ResponsesContinuation::new(uuid::Uuid::new_v4(), model.clone(), 16384).unwrap());
    let caps = ModelCapabilities {
        vision: true,
        tools: true,
        parallel_tools: false,
        reasoning_efforts: effort.clone().into_iter().collect(),
    };
    let transport = prepared.into_chatgpt_transport(caps, state).unwrap();
    assert_eq!(source.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    let result = transport
        .start(ModelRequest {
            captured_history: None,
            request_id: uuid::Uuid::new_v4(),
            model,
            messages: vec![ModelMessage::User {
                content: vec![ContentPart::Text("fixture".into())],
                source_item_id: None,
            }],
            tools: vec![],
            max_output_tokens: 128,
            parallel_tool_calls: false,
            reasoning_effort: effort,
        })
        .await;
    assert!(matches!(result, Err(ModelError::Authentication)));
    assert_eq!(source.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    fixture
        .resolver
        .connection
        .lock()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=0 where provider_id='openai'",
            [],
        )
        .unwrap();
    assert!(matches!(
        fixture
            .resolver
            .prepare(AgentHost::Writing, selection("openai", "fixture-model"))
            .await,
        Err(AgentModelPreparationError::Disabled)
    ));
}

#[tokio::test]
async fn fast_mode_is_resolved_only_for_supported_openai_models() {
    let mut fixture = fixture();
    fixture.resolver.chatgpt = Some(Arc::new(UnavailableChatGpt(
        std::sync::atomic::AtomicUsize::new(0),
    )));
    for (provider, model, tier, allowed) in [
        ("openai", "gpt-5.6-terra", "fast", true),
        ("openai", "gpt-5.6-terra", "default", true),
        ("openai", "fixture-model", "fast", false),
        ("openai", "gpt-5.6-terra", "flex", false),
        ("deepseek", "global-model", "fast", false),
    ] {
        let mut choice = selection(provider, model).unwrap();
        choice.service_tier = Some(tier.into());
        let prepared = fixture
            .resolver
            .prepare(AgentHost::Writing, Some(choice))
            .await;
        if allowed {
            assert_eq!(prepared.unwrap().service_tier(), Some(tier));
        } else {
            assert!(matches!(
                prepared,
                Err(AgentModelPreparationError::Settings)
            ));
        }
    }
}
