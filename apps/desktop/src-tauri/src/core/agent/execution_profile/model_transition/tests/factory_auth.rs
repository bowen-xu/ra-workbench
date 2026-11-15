//! Actual resolver/profile composition; unavailable credentials must prevent I/O.
use super::*;
use crate::agent::{
    authentication::{AuthenticationError, ChatGptCredentialSource, CredentialFuture},
    repository::{SqliteSubmissionStore, create_conversation},
};
use research_agent_core::{conversation::ConversationId, submission::*};
use std::sync::atomic::{AtomicUsize, Ordering};

struct UnavailableCredentials(AtomicUsize);
impl ChatGptCredentialSource for UnavailableCredentials {
    fn acquire(&self) -> CredentialFuture<'_> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(AuthenticationError::Unavailable) })
    }
}

#[tokio::test]
async fn native_factory_preserves_auth_failure_for_hash_transition_without_remote_fallback() {
    for previous in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        for current in [
            NativeCompactionPolicy::ChatGptResponsesV2,
            NativeCompactionPolicy::ChatGptResponsesFullWindow,
        ] {
            exercise(previous, current).await;
        }
    }
}

async fn exercise(previous_native: NativeCompactionPolicy, current_native: NativeCompactionPolicy) {
    let root = tempfile::tempdir().unwrap();
    let mut db = rusqlite::Connection::open(root.path().join("factory.sqlite")).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let conversation = uuid::Uuid::new_v4();
    create_conversation(&mut db, ConversationId::from_uuid(conversation), "Factory").unwrap();
    db.execute(
        "update llm_provider_configs set enabled=1,base_url='' where provider_id='openai'",
        [],
    )
    .unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let credentials = Arc::new(UnavailableCredentials(AtomicUsize::new(0)));
    let resolver = AgentModelResolver::from_app_store(
        &store,
        crate::ai::JsonCredentialStore::from_path(root.path().join("synthetic.json")),
    )
    .with_chatgpt(credentials.clone());
    let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
    let mut snapshots = vec![];
    let mut records = vec![];
    for hash in ["old-compatible-state", "new-compatible-state"] {
        let mut frozen = policy("gpt-5.6-terra", None);
        frozen.binding.model.provider_id = "openai".into();
        if !records.is_empty() {
            frozen.binding.model.revision = Some("new-revision".into());
        }
        frozen.binding = if records.is_empty() {
            previous_native
        } else {
            current_native
        }
        .binding(frozen.binding.model.clone());
        frozen.authentication = ContextAuthentication::CodexBackend;
        frozen.comp_hash = Some(hash.into());
        frozen.budget = Some(CompactionBudget {
            context_window_tokens: if records.is_empty() { 4000 } else { 2000 },
            effective_percent: 95,
            configured_trigger_tokens: None,
            output_reserve_tokens: 100,
        });
        let record = submissions
            .put(
                conversation,
                uuid::Uuid::new_v4(),
                SubmissionInput {
                    remote_compaction_v2: None,
                    user_input: vec![],
                    skill_instructions: vec![],
                    instructions: vec![],
                    instruction_configs: vec![],
                    skills: vec![],
                    text: "synthetic evidence".into(),
                    model: frozen.binding.model.clone(),
                    reasoning_effort: None,
                    service_tier: None,
                    context: vec![],
                },
            )
            .await
            .unwrap();
        let snapshot =
            SqliteContextPolicy::new(store.shared_agent_connection(), &record, frozen).unwrap();
        snapshot.prepare().await.unwrap();
        snapshots.push(snapshot);
        records.push(record);
    }
    let history = ModelHistory {
        store,
        models: resolver,
        host: AgentHost::Writing,
    };
    let previous = &records[0];
    let strategy = history
        .select(&snapshots[1], previous.turn_id, &previous.input.model, None)
        .await
        .unwrap()
        .unwrap();
    // Preparation uses the real resolver, but does not acquire credentials or infer.
    assert_eq!(credentials.0.load(Ordering::SeqCst), 0);
    assert_eq!(strategy.execution_model(), previous.input.model);
    assert_eq!(strategy.binding(), snapshots[1].snapshot().binding);
    let accounting = strategy.input_accounting().unwrap();
    assert_eq!(accounting.model, previous.input.model);
    assert_eq!(accounting.budget, snapshots[0].snapshot().budget);
    assert_ne!(accounting.budget, snapshots[1].snapshot().budget);
    assert_eq!(accounting.budget.unwrap().context_window_tokens, 4000);
    let attempt = uuid::Uuid::new_v4();
    let failure = prepare_compaction(
        strategy.as_ref(),
        CompactionRequest {
            operation_id: attempt,
            conversation_id: conversation,
            turn_id: records[1].turn_id,
            history_revision: 0,
            phase: CompactionPhase::PreTurn,
            binding: strategy.binding(),
            max_result_bytes: 8192,
            source: ModelRequest {
                captured_history: None,
                request_id: attempt,
                model: records[1].input.model.clone(),
                messages: vec![ModelMessage::User {
                    content: vec![ContentPart::Text("synthetic evidence".into())],
                    source_item_id: None,
                }],
                tools: vec![],
                max_output_tokens: 64,
                parallel_tool_calls: false,
                reasoning_effort: None,
            },
        },
    )
    .await;
    assert!(matches!(
        failure,
        Err(CompactionFailure::Model(ModelError::Authentication))
    ));
    assert_eq!(credentials.0.load(Ordering::SeqCst), 1);
    assert!(!strategy.allows_current_model_fallback(&ModelError::Authentication));
    assert!(!strategy.allows_current_model_fallback(&ModelError::ContextWindowExceeded));
    assert_eq!(
        snapshots[0].snapshot().comp_hash.as_deref(),
        Some("old-compatible-state")
    );
    let restored =
        SqliteContextPolicy::restored(&history.store.shared_agent_connection(), previous)
            .unwrap()
            .unwrap();
    assert_eq!(
        restored.binding,
        previous_native.binding(previous.input.model.clone())
    );
}

#[cfg(all(debug_assertions, feature = "development-data"))]
#[tokio::test]
async fn previous_model_uses_current_codec_without_changing_its_saved_policy() {
    use crate::agent::model::responses_transport::PreviewResponsesWire;
    use std::sync::Mutex;

    struct Wire {
        paths: Mutex<Vec<String>>,
        models: Mutex<Vec<String>>,
        v2: bool,
    }
    impl PreviewResponsesWire for Wire {
        fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
            self.paths
                .lock()
                .unwrap()
                .push(request.url().path().to_owned());
            let body: serde_json::Value =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            self.models
                .lock()
                .unwrap()
                .push(body["model"].as_str().unwrap().to_owned());
            let item = serde_json::json!({"type":"compaction","id":"synthetic","encrypted_content":"synthetic"});
            let (mime, body) = if self.v2 {
                ("text/event-stream", [
                    serde_json::json!({"type":"response.created","response":{"id":"synthetic-response"}}),
                    serde_json::json!({"type":"response.output_item.done","item":item}),
                    serde_json::json!({"type":"response.completed","response":{"id":"synthetic-response","status":"completed"}}),
                ].into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>())
            } else {
                (
                    "application/json",
                    serde_json::json!({"output":[item]}).to_string(),
                )
            };
            Box::pin(async move {
                Ok(hyper::Response::builder()
                    .status(200)
                    .header("content-type", mime)
                    .body(reqwest::Body::from(body))
                    .unwrap()
                    .into())
            })
        }
    }
    for (previous_native, current_native) in [
        (
            NativeCompactionPolicy::ChatGptResponsesV2,
            NativeCompactionPolicy::ChatGptResponsesFullWindow,
        ),
        (
            NativeCompactionPolicy::ChatGptResponsesFullWindow,
            NativeCompactionPolicy::ChatGptResponsesV2,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut db = rusqlite::Connection::open(root.path().join("transition.sqlite")).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let conversation = uuid::Uuid::new_v4();
        create_conversation(
            &mut db,
            ConversationId::from_uuid(conversation),
            "Transition",
        )
        .unwrap();
        let store = AppStore::for_test(db, root.path().join("objects"));
        let wire = Arc::new(Wire {
            paths: Mutex::new(vec![]),
            models: Mutex::new(vec![]),
            v2: current_native == NativeCompactionPolicy::ChatGptResponsesV2,
        });
        let selected_wire = wire.clone();
        let resolver = AgentModelResolver::for_preview(
            &store,
            Arc::new(move |_, selection| {
                let selection: AgentModelSelection =
                    selection.ok_or(AgentModelPreparationError::Settings)?;
                PreparedAgentModel::for_preview(
                    ModelIdentity {
                        provider_id: selection.provider_id,
                        model_id: selection.model_id,
                        revision: None,
                    },
                    selection.reasoning_effort,
                    selection.service_tier,
                    ModelCapabilities {
                        vision: false,
                        tools: true,
                        parallel_tools: false,
                        reasoning_efforts: vec![],
                    },
                    selected_wire.clone(),
                    uuid::Uuid::from_u128(123),
                )
            }),
        )
        .unwrap();
        let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
        let mut records = vec![];
        let mut policies = vec![];
        for (native, hash) in [(previous_native, "old"), (current_native, "new")] {
            let mut frozen = policy("gpt-5.6-terra", None);
            frozen.binding.model.provider_id = "openai".into();
            if !records.is_empty() {
                frozen.binding.model.model_id = "gpt-5.6-sol".into();
            }
            frozen.binding = native.binding(frozen.binding.model.clone());
            frozen.authentication = ContextAuthentication::CodexBackend;
            frozen.comp_hash = Some(hash.into());
            let record = submissions
                .put(
                    conversation,
                    uuid::Uuid::new_v4(),
                    SubmissionInput {
                        remote_compaction_v2: None,
                        user_input: vec![],
                        skill_instructions: vec![],
                        instructions: vec![],
                        instruction_configs: vec![],
                        skills: vec![],
                        text: "synthetic evidence".into(),
                        model: frozen.binding.model.clone(),
                        reasoning_effort: None,
                        service_tier: None,
                        context: vec![],
                    },
                )
                .await
                .unwrap();
            let saved =
                SqliteContextPolicy::new(store.shared_agent_connection(), &record, frozen).unwrap();
            saved.prepare().await.unwrap();
            records.push(record);
            policies.push(saved);
        }
        let history = ModelHistory {
            store,
            models: resolver,
            host: AgentHost::Writing,
        };
        let selected = history
            .select(
                &policies[1],
                records[0].turn_id,
                &records[0].input.model,
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let operation = uuid::Uuid::new_v4();
        let candidate = prepare_compaction(
            selected.as_ref(),
            CompactionRequest {
                operation_id: operation,
                conversation_id: conversation,
                turn_id: records[1].turn_id,
                history_revision: 0,
                phase: CompactionPhase::PreTurn,
                binding: selected.binding(),
                max_result_bytes: 8192,
                source: ModelRequest {
                    captured_history: None,
                    request_id: operation,
                    model: records[1].input.model.clone(),
                    messages: vec![ModelMessage::User {
                        content: vec![ContentPart::Text("synthetic evidence".into())],
                        source_item_id: None,
                    }],
                    tools: vec![],
                    max_output_tokens: 64,
                    parallel_tool_calls: false,
                    reasoning_effort: None,
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(
            candidate.binding,
            current_native.binding(records[1].input.model.clone())
        );
        assert!(matches!(
            (&candidate.output, current_native),
            (
                CompactionOutput::RemoteOpaque(_),
                NativeCompactionPolicy::ChatGptResponsesV2
            ) | (
                CompactionOutput::RemoteWindow(_),
                NativeCompactionPolicy::ChatGptResponsesFullWindow
            )
        ));
        let expected_path = if wire.v2 {
            "/responses"
        } else {
            "/responses/compact"
        };
        assert_eq!(wire.paths.lock().unwrap().as_slice(), &[expected_path]);
        assert_eq!(wire.models.lock().unwrap().as_slice(), &["gpt-5.6-terra"]);
        assert_eq!(
            SqliteContextPolicy::restored(&history.store.shared_agent_connection(), &records[0])
                .unwrap()
                .unwrap()
                .binding,
            previous_native.binding(records[0].input.model.clone())
        );
    }
}
