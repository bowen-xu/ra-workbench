use super::*;
#[path = "tests/accounting.rs"]
mod accounting;
#[path = "tests/cancellation_tools.rs"]
mod cancellation_tools;
#[path = "tests/compaction_cancel.rs"]
mod compaction_cancel;
#[path = "tests/compaction_fallback.rs"]
mod compaction_fallback;
#[path = "tests/compaction_runtime.rs"]
mod compaction_runtime;
#[path = "tests/context_window.rs"]
mod context_window;
#[path = "tests/fork.rs"]
mod fork;
#[path = "tests/images.rs"]
mod images;
#[path = "tests/manual_compaction.rs"]
mod manual_compaction;
#[path = "tests/sampling_skills.rs"]
mod sampling_skills;
#[path = "tests/unknown_names.rs"]
mod unknown_names;
use crate::agent::{
    mcp_config::{ConfiguredServer, Transport as McpTransport},
    mcp_runtime::McpRuntime,
    repository::{SqliteConversationStore, SqliteSubmissionStore},
    tasks::TaskCompletion,
};
use research_agent_core::{
    protocol::ItemKind,
    submission::{ContextSourceKind, ContextText, SubmissionInput},
};
use rusqlite::Connection;
use std::{
    collections::BTreeMap,
    collections::VecDeque,
    ffi::OsString,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Transport {
    requests: Mutex<Vec<ModelRequest>>,
    responses: Mutex<VecDeque<Vec<ModelStreamEvent>>>,
}
struct Stream(VecDeque<ModelStreamEvent>);

// Tests exercise the real token decision rather than the removed byte trigger.
struct TestCompactionTransport {
    inner: Arc<dyn ModelTransport>,
    trigger: u64,
}
impl ModelTransport for TestCompactionTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn context_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        use research_agent_core::context::accounting::{ContextAccounting, Utf8JsonTokenEstimator};
        Some(Arc::new(ContextAccounting {
            model: model(),
            budget: None,
            unknown_window_trigger_tokens: Some(self.trigger),
            estimator: Arc::new(Utf8JsonTokenEstimator),
        }))
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
    fn start_text_compaction(
        &self,
        request: ModelRequest,
    ) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start_text_compaction(request)
    }
}
fn token_compaction(inner: Arc<dyn ModelTransport>, trigger: u64) -> Arc<dyn ModelTransport> {
    Arc::new(TestCompactionTransport { inner, trigger })
}
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
impl ModelTransport for Transport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec!["medium".into()],
        }
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            let events = self.responses.lock().unwrap().pop_front().unwrap();
            Ok(Box::new(Stream(events.into())) as Box<dyn ModelStream>)
        })
    }
}
struct Tools(AtomicUsize);
impl ToolExecutor for Tools {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if call.name == "read" {
            Ok(())
        } else {
            Err(RunError::ToolRejected)
        }
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolOutput {
                content: "tool evidence".into(),
                is_error: false,
            })
        })
    }
}
fn model() -> ModelIdentity {
    ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "test".into(),
        revision: None,
    }
}
fn response(tool: bool) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Started {
        response_id: "fixture-response".into(),
        actual_model: model(),
    }];
    if tool {
        events.extend([
            ModelStreamEvent::ToolCallStarted {
                call_id: "read-1".into(),
                name: "read".into(),
            },
            ModelStreamEvent::ToolArgumentsDelta {
                call_id: "read-1".into(),
                delta: "{}".into(),
            },
            ModelStreamEvent::ToolCallCompleted {
                call_id: "read-1".into(),
            },
            ModelStreamEvent::Completed {
                reason: FinishReason::ToolCalls,
                end_turn: None,
            },
        ]);
    } else {
        events.extend([
            ModelStreamEvent::TextDelta("answer".into()),
            ModelStreamEvent::Completed {
                reason: FinishReason::Stop,
                end_turn: None,
            },
        ]);
    }
    events
}
fn text_strategy(
    transport: Arc<dyn ModelTransport>,
) -> Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy> {
    use research_agent_core::context::{
        compaction_strategy::{CompactionBinding, CompactionKind},
        text_compaction::TextCompactionStrategy,
    };
    Arc::new(
        TextCompactionStrategy::new(
            transport,
            CompactionBinding {
                model: model(),
                strategy_id: "research.text.v1".into(),
                state_schema: 1,
                kind: CompactionKind::TextSummary,
            },
        )
        .unwrap(),
    )
}
type ExecutionFixture = (
    Arc<AgentSessionService>,
    Arc<SqliteSubmissionStore>,
    Arc<Transport>,
    Arc<Tools>,
    AgentExecution,
    Uuid,
);
async fn setup() -> ExecutionFixture {
    setup_with_connection().await.0
}
async fn setup_with_connection() -> (ExecutionFixture, crate::data_interface::SharedDatabase) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let app = crate::storage::AppStore::for_test(db, directory.path().join("objects"));
    setup_with_app(&app).await
}
async fn setup_with_app(
    app: &crate::storage::AppStore,
) -> (ExecutionFixture, crate::data_interface::SharedDatabase) {
    let db = app.shared_agent_connection();
    let service = Arc::new(AgentSessionService::from_app_store(app).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let submissions = Arc::new(SqliteSubmissionStore::new(db.clone()));
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(vec![response(true), response(false), response(false)].into()),
    });
    let tools = Arc::new(Tools(AtomicUsize::new(0)));
    let execution = AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        model: model(),
        compaction_strategy: Some(text_strategy(transport.clone())),
        transport: transport.clone(),
        host_tools: research_agent_core::tool_registry::OwnedToolRegistration::new(
            vec![ToolDefinition {
                namespace: None,
                name: "read".into(),
                description: "Read authorized source".into(),
                parameters: serde_json::json!({"type":"object"}),
            }],
            tools.clone(),
        ),
        system_instructions: "Research assistant".into(),
        max_output_tokens: 100,
        max_input_bytes: 100_000,
        limits: RunLimits {
            model_calls: Some(3),
            tool_calls: Some(3),
            output_bytes: Some(10_000),
            tool_output_bytes: 10_000,
            response: StreamLimits {
                text_bytes: 10_000,
                argument_bytes: 10_000,
                tool_calls: 3,
            },
        },
    };
    ((service, submissions, transport, tools, execution, id), db)
}
fn input(version: &str) -> SubmissionInput {
    SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: "Explain".into(),
        model: model(),
        reasoning_effort: None,
        service_tier: None,
        context: vec![ContextText {
            source_id: "S1".into(),
            kind: ContextSourceKind::Manuscript,
            resource_id: "draft/main.tex".into(),
            revision: version.into(),
            anchor: None,
            text: format!("evidence {version}"),
        }],
    }
}
async fn run(
    execution: Arc<AgentExecution>,
    service: Arc<AgentSessionService>,
    store: Arc<SqliteSubmissionStore>,
    id: Uuid,
    request: Uuid,
) -> TaskCompletion {
    let admission = execution
        .start(service, store, ConversationId::from_uuid(id), request)
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    owned.task.wait().await
}

#[tokio::test]
async fn persisted_input_runs_model_tool_model_and_restores_prior_source_versions() {
    let (service, store, transport, tools, execution, id) = setup().await;
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let second = store.put(id, Uuid::new_v4(), input("v2")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            second.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        let first_tool_names: Vec<_> = requests[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        for built_in in [
            "research_search_sources",
            "research_list_sources",
            "research_read_source",
        ] {
            assert_eq!(
                first_tool_names
                    .iter()
                    .filter(|name| **name == built_in)
                    .count(),
                1,
                "built-in source tool {built_in} must have one definition owner"
            );
        }
        let texts: Vec<_> = requests[2]
            .messages
            .iter()
            .flat_map(|message| match message {
                ModelMessage::User { content: parts, .. } => parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            })
            .collect();
        assert!(texts.iter().any(|text| text.contains("evidence v1")));
        assert!(texts.iter().any(|text| text.contains("evidence v2")));
        assert!(requests[2].messages.iter().any(|message| matches!(message, ModelMessage::ToolResult { content, .. } if content == "tool evidence")));
    }
    let retry = execution
        .start(
            service,
            store,
            ConversationId::from_uuid(id),
            first.request_id,
        )
        .await
        .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn preparation_failure_persists_a_diagnostic_model_attempt() {
    let (service, store, _transport, _tools, mut execution, id) = setup().await;
    execution.max_input_bytes = 1;
    let execution = Arc::new(execution);
    let request = store
        .put(id, Uuid::new_v4(), input("diagnostic"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, request.request_id).await,
        TaskCompletion::Failed
    );
    let projection = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = projection.turn(request.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
    assert_eq!(turn.attempt_order.len(), 1);
    let attempt = turn.attempts.get(&turn.attempt_order[0]).unwrap();
    assert_eq!(attempt.outcome, Some(TurnOutcome::Failed));
    assert_eq!(attempt.actual_model, None);
    assert_eq!(
        attempt.failure_category,
        Some(research_agent_core::protocol::FailureCategory::Admission)
    );
}

#[tokio::test]
async fn unsupported_reasoning_is_failed_without_model_calls() {
    let (service, store, transport, _, execution, id) = setup().await;
    let mut draft = input("v1");
    draft.reasoning_effort = Some("high".into());
    let record = store.put(id, Uuid::new_v4(), draft).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            record.request_id
        )
        .await,
        TaskCompletion::Failed
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Failed)
    );
}

#[tokio::test]
async fn duplicate_host_and_builtin_tool_names_fail_before_transport() {
    let (service, store, transport, tools, mut execution, id) = setup().await;
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![
            ToolDefinition {
                namespace: None,
                name: "read".into(),
                description: "Read authorized source".into(),
                parameters: serde_json::json!({"type":"object"}),
            },
            ToolDefinition {
                namespace: None,
                name: "research_read_source".into(),
                description: "Invalid duplicate host registration".into(),
                parameters: serde_json::json!({"type":"object"}),
            },
        ],
        tools,
    );
    let record = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(Arc::new(execution), service, store, id, record.request_id).await,
        TaskCompletion::Failed
    );
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn input_budget_failure_is_terminal_and_does_not_send_partial_context() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    execution.max_input_bytes = 1;
    let record = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(Arc::new(execution), service, store, id, record.request_id).await,
        TaskCompletion::Failed
    );
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[test]
fn input_budget_counts_tool_schemas_and_historical_arguments() {
    let tools = vec![ToolDefinition {
        namespace: None,
        name: "read".into(),
        description: "d".into(),
        parameters: serde_json::json!({"x":"a".repeat(100)}),
    }];
    assert!(matches!(
        check_budget(&[], &tools, 50),
        Err(RunError::Budget)
    ));
    let messages = vec![ModelMessage::Assistant {
        phase: None,
        content: vec![],
        tool_calls: vec![ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "a".into(),
            name: "read".into(),
            arguments: serde_json::json!({"x":"a".repeat(100)}),
        }],
    }];
    assert!(matches!(
        check_budget(&messages, &[], 50),
        Err(RunError::Budget)
    ));
}

struct MissingHistory(Arc<SqliteSubmissionStore>);
impl SubmissionStore for MissingHistory {
    fn get_by_turn(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> research_agent_core::submission::SubmissionFuture<'_, SubmissionRecord> {
        Box::pin(async { Err(research_agent_core::submission::SubmissionError::NotFound) })
    }
    fn get(
        &self,
        id: Uuid,
        request: Uuid,
    ) -> research_agent_core::submission::SubmissionFuture<'_, SubmissionRecord> {
        self.0.get(id, request)
    }
    fn put(
        &self,
        id: Uuid,
        request: Uuid,
        input: SubmissionInput,
    ) -> research_agent_core::submission::SubmissionFuture<'_, SubmissionRecord> {
        self.0.put(id, request, input)
    }
}
#[tokio::test]
async fn missing_historical_snapshot_fails_instead_of_sending_incomplete_evidence() {
    let (service, store, transport, _, execution, id) = setup().await;
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let second = store.put(id, Uuid::new_v4(), input("v2")).await.unwrap();
    let admission = execution
        .start(
            service,
            Arc::new(MissingHistory(store)),
            ConversationId::from_uuid(id),
            second.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Failed);
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn saved_reasoning_is_retained_through_tool_continuation() {
    let (service, store, transport, _, execution, id) = setup().await;
    let mut draft = input("v1");
    draft.reasoning_effort = Some("medium".into());
    let record = store.put(id, Uuid::new_v4(), draft).await.unwrap();
    assert_eq!(
        run(Arc::new(execution), service, store, id, record.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|r| r.reasoning_effort.as_deref() == Some("medium"))
    );
}

#[tokio::test]
async fn runner_searches_confirmed_sources_and_continues_with_the_result() {
    let (service, store, transport, fallback, execution, id) = setup().await;
    {
        let mut responses = transport.responses.lock().unwrap();
        for event in responses.front_mut().unwrap() {
            match event {
                ModelStreamEvent::ToolCallStarted { name, .. } => {
                    *name = "research_search_sources".into()
                }
                ModelStreamEvent::ToolArgumentsDelta { delta, .. } => {
                    *delta = r#"{"query":"evidence"}"#.into()
                }
                _ => {}
            }
        }
    }
    let record = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(Arc::new(execution), service, store, id, record.request_id).await,
        TaskCompletion::Completed
    );
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "research_search_sources")
    );
    let result = requests[1]
        .messages
        .iter()
        .find_map(|message| match message {
            ModelMessage::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(result).unwrap();
    assert_eq!(
        value["contextSnapshotId"],
        record.context_snapshot_id.to_string()
    );
    assert_eq!(value["total"], 1);
    assert_eq!(value["matches"][0]["sourceId"], "S1");
    assert_eq!(value["matches"][0]["startChar"], 0);
}

#[tokio::test]
async fn runner_reads_frozen_source_through_registered_tool_and_persists_result() {
    let (service, store, transport, fallback, execution, id) = setup().await;
    {
        let mut responses = transport.responses.lock().unwrap();
        for event in responses.front_mut().unwrap() {
            match event {
                ModelStreamEvent::ToolCallStarted { name, .. } => {
                    *name = "research_read_source".into()
                }
                ModelStreamEvent::ToolArgumentsDelta { delta, .. } => {
                    *delta = r#"{"sourceId":"S1"}"#.into()
                }
                _ => {}
            }
        }
    }
    let record = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            record.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
    {
        let requests = transport.requests.lock().unwrap();
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == "research_read_source")
        );
        let result = requests[1]
            .messages
            .iter()
            .find_map(|message| match message {
                ModelMessage::ToolResult { content, .. } => Some(content),
                _ => None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(result).unwrap();
        assert_eq!(value["text"], "evidence v1");
        assert_eq!(
            value["contextSnapshotId"],
            record.context_snapshot_id.to_string()
        );
    }
    let page = service
        .read_since(ConversationId::from_uuid(id), 0, 100)
        .await
        .unwrap();
    assert!(page.items.iter().any(|event| matches!(event.payload(), EventPayload::TextDelta { text, .. } if text.contains("evidence v1") && text.contains(&record.context_snapshot_id.to_string()))));
    assert!(!page.items.iter().any(|event| matches!(
        event.payload(),
        EventPayload::ToolReceiptPending { .. } | EventPayload::ToolReceiptConfirmed { .. }
    )));
}

#[tokio::test]
async fn shared_runner_executes_discovered_mcp_tool_and_continues() {
    let root = tempfile::tempdir().unwrap();
    let script = r#"while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"search","inputSchema":{"type":"object"}}]}}' ;; *tools/call*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"mcp evidence"}]}}' ;; esac; done"#;
    let runtime = McpRuntime::build(
        vec![ConfiguredServer {
            server_id: "literature".into(),
            environment_id: "runner-test".into(),
            revision: "r1".into(),
            transport: McpTransport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from(script)],
                directory: root.path().to_path_buf(),
            },
            environment: BTreeMap::new(),
            credential_handle: None,
            enabled: true,
        }],
        &std::collections::HashSet::from(["literature".to_owned()]),
        // This checks discovery and runner continuation, not response latency.
        // Allow scheduling under the full suite's concurrent migration work;
        // timeout/late-response rejection is covered by mcp_stdio tests.
        std::time::Duration::from_secs(15),
    )
    .await
    .unwrap();
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([
            vec![
                ModelStreamEvent::Started {
                    response_id: "mcp-first".into(),
                    actual_model: model(),
                },
                ModelStreamEvent::ToolCallStarted {
                    call_id: "mcp-call".into(),
                    name: "literature__search".into(),
                },
                ModelStreamEvent::ToolArgumentsDelta {
                    call_id: "mcp-call".into(),
                    delta: "{}".into(),
                },
                ModelStreamEvent::ToolCallCompleted {
                    call_id: "mcp-call".into(),
                },
                ModelStreamEvent::Completed {
                    reason: FinishReason::ToolCalls,
                    end_turn: None,
                },
            ],
            vec![
                ModelStreamEvent::Started {
                    response_id: "mcp-answer".into(),
                    actual_model: model(),
                },
                ModelStreamEvent::TextDelta("mcp continuation complete".into()),
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                },
            ],
        ])),
    });
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let db = Arc::new(Mutex::new(db));
    let service = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db.clone(), "mcp-runner-fixture".into()).unwrap(),
    )));
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let submissions = Arc::new(SqliteSubmissionStore::new(db));
    let record = submissions
        .put(id, Uuid::new_v4(), input("mcp-v1"))
        .await
        .unwrap();
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        model: model(),
        compaction_strategy: Some(text_strategy(transport.clone())),
        transport,
        host_tools: research_agent_core::tool_registry::OwnedToolRegistration::new(
            runtime.definitions(),
            Arc::new(runtime),
        ),
        system_instructions: "Use the registered MCP tool once, then answer.".into(),
        limits: RunLimits {
            model_calls: Some(2),
            tool_calls: Some(1),
            output_bytes: Some(4096),
            tool_output_bytes: 4096,
            response: StreamLimits {
                text_bytes: 4096,
                argument_bytes: 1024,
                tool_calls: 1,
            },
        },
        max_output_tokens: 128,
        max_input_bytes: 32 * 1024,
    });
    assert_eq!(
        run(
            execution,
            service.clone(),
            submissions,
            id,
            record.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let projection = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = projection.turn(record.turn_id).unwrap();
    assert!(turn.items.values().any(|item| {
        matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == "literature__search")
    }));
    assert!(turn.items.values().any(|item| {
        matches!(
            &item.kind,
            ItemKind::ToolResult {
                is_error: Some(false),
                ..
            }
        ) && item.text.contains("mcp evidence")
    }));
    assert!(turn.items.values().any(|item| {
        matches!(item.kind, ItemKind::AssistantMessage)
            && item.text.contains("mcp continuation complete")
    }));
}

#[path = "tests/instructions.rs"]
mod instructions;
#[path = "tests/resource_context.rs"]
mod resource_context;
