use super::*;
#[path = "accounting/tail_trim.rs"]
mod tail_trim;
use research_agent_core::context::{
    accounting::ContextAccounting, compaction_strategy::CompactionBudget,
};

struct Accounted {
    inner: Arc<ResponsesTransport>,
    policy: Arc<ContextAccounting>,
}
impl ModelTransport for Accounted {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn supports_compaction_kind(&self, kind: CompactionKind) -> bool {
        self.inner.supports_compaction_kind(kind)
    }
    fn validate_compaction_window<'a>(
        &'a self,
        checkpoint: &'a ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        self.inner.validate_compaction_window(checkpoint, request)
    }
    fn context_accounting(&self) -> Option<Arc<ContextAccounting>> {
        Some(self.policy.clone())
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}
pub(super) fn account(execution: &mut AgentExecution, inner: Arc<ResponsesTransport>, window: u64) {
    execution.transport = Arc::new(Accounted {
        inner,
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: execution.model.clone(),
            budget: Some(CompactionBudget {
                context_window_tokens: window,
                effective_percent: 95,
                configured_trigger_tokens: None,
                output_reserve_tokens: u64::from(execution.max_output_tokens),
            }),
            estimator: Arc::new(
                research_agent_core::context::accounting::ServerObservedEstimator::new(Arc::new(
                    crate::agent::model::responses_accounting::ResponsesContextEstimator,
                )),
            ),
        }),
    });
}

struct ToolResultEstimate;
impl research_agent_core::context::accounting::ContextTokenEstimator for ToolResultEstimate {
    fn estimate(
        &self,
        messages: &[ModelMessage],
        _: &[research_agent_core::model::ToolDefinition],
    ) -> Option<u64> {
        Some(if messages.iter().any(|message| matches!(message, ModelMessage::ToolResult { content, .. } if content.len() >= 8_000)) { 3_000 } else { 100 })
    }
    fn estimate_compaction(
        &self,
        _: &research_agent_core::context::compaction_strategy::CompactionOutput,
    ) -> Option<u64> {
        Some(0)
    }
}

struct OldHistoryEstimate;
impl research_agent_core::context::accounting::ContextTokenEstimator for OldHistoryEstimate {
    fn estimate(
        &self,
        messages: &[ModelMessage],
        _: &[research_agent_core::model::ToolDefinition],
    ) -> Option<u64> {
        Some(if messages.iter().any(|message| matches!(message, ModelMessage::User { content, .. } if content.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("old user evidence"))))) { 3_000 } else { 100 })
    }
    fn estimate_compaction(
        &self,
        _: &research_agent_core::context::compaction_strategy::CompactionOutput,
    ) -> Option<u64> {
        Some(0)
    }
    fn estimate_window(
        &self,
        messages: &[ModelMessage],
        tools: &[research_agent_core::model::ToolDefinition],
        checkpoint: Option<&ContextCheckpoint>,
    ) -> Result<Option<u64>, ModelError> {
        if checkpoint.is_some() {
            Ok(Some(100))
        } else {
            Ok(self.estimate(messages, tools))
        }
    }
}
pub(super) fn account_for_old_history(
    execution: &mut AgentExecution,
    inner: Arc<ResponsesTransport>,
) {
    execution.transport = Arc::new(Accounted {
        inner,
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: execution.model.clone(),
            budget: Some(CompactionBudget {
                context_window_tokens: 10_000,
                effective_percent: 95,
                configured_trigger_tokens: Some(2_000),
                output_reserve_tokens: u64::from(execution.max_output_tokens),
            }),
            estimator: Arc::new(OldHistoryEstimate),
        }),
    });
}
pub(super) fn account_for_tool_result(
    execution: &mut AgentExecution,
    inner: Arc<ResponsesTransport>,
) {
    execution.transport = Arc::new(Accounted {
        inner,
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: execution.model.clone(),
            budget: Some(CompactionBudget {
                context_window_tokens: 10_000,
                effective_percent: 95,
                configured_trigger_tokens: Some(2_000),
                output_reserve_tokens: u64::from(execution.max_output_tokens),
            }),
            estimator: Arc::new(ToolResultEstimate),
        }),
    });
}

struct SmallTool;
impl ToolExecutor for SmallTool {
    fn validate(&self, _: &research_agent_core::model::ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(
        &self,
        _: research_agent_core::model::ToolCall,
    ) -> RunFuture<'_, ToolOutput> {
        Box::pin(async {
            Ok(ToolOutput {
                content: "small evidence".into(),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn server_usage_triggers_remote_midturn_below_local_estimate_and_byte_guard() {
    let fixture = Fixture::new().await;
    let call = json!({"type":"function_call","id":"read-item","call_id":"read1","name":"read","arguments":"{}"});
    let frames = [
        json!({"type":"response.created","response":{"id":"r1","model":"fixture-model","headers":{"OpenAI-Model":"fixture-model"}}}),
        json!({"type":"response.output_item.done","item":call}),
        json!({"type":"response.completed","response":{"id":"r1","model":"fixture-model","status":"completed",
            "output":[call],"usage":{"input_tokens":1500,"output_tokens":400}}}),
    ].into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
    let (url, peer) = sequential_server(vec![
        ok(&frames),
        ok(&events(&[opaque()], true)),
        ok(&body()),
    ])
    .await;
    let record = fixture.record("small submission").await;
    let inner = fixture.transport(&record, &url);
    let mut execution = fixture.execution(&record, inner.clone());
    account(&mut execution, inner, 2000);
    execution.host_tools = OwnedToolRegistration::new(
        vec![research_agent_core::model::ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized evidence".into(),
            parameters: json!({"type":"object"}),
        }],
        Arc::new(SmallTool),
    );
    assert_eq!(
        fixture
            .start(&record, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let window = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(window.phase, CompactionPhase::MidTurn);
    let requests = peer.await.unwrap();
    assert!(
        requests[1]["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "compaction_trigger")
    );
    assert!(requests[2]["input"].as_array().unwrap().contains(&opaque()));
}
fn large_state() -> Value {
    json!({"type":"compaction", "encrypted_content":"a".repeat(12000)})
}

#[tokio::test]
async fn opaque_candidate_over_token_budget_is_not_installed() {
    let fixture = Fixture::new().await;
    let (url, peer) = server(ok(&events(&[large_state()], true)), false).await;
    let record = fixture.record("/compact").await;
    let inner = fixture.transport(&record, &url);
    let mut execution = fixture.execution(&record, inner.clone());
    account(&mut execution, inner, 2000);
    assert_eq!(
        fixture
            .start(&record, execution, true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Failed
    );
    assert!(
        fixture
            .service
            .active_context_checkpoint(fixture.id)
            .await
            .unwrap()
            .is_none()
    );
    peer.await.unwrap();
}

#[tokio::test]
async fn installed_opaque_cost_triggers_pre_turn_compaction_below_normalized_byte_guard() {
    let fixture = Fixture::new().await;
    let large = large_state();
    let (url, peer) = sequential_server(vec![
        ok(&events(std::slice::from_ref(&large), true)),
        ok(&events(&[opaque()], true)),
        ok(&body()),
    ])
    .await;
    let record = fixture.record("/compact").await;
    let inner = fixture.transport(&record, &url);
    let mut execution = fixture.execution(&record, inner.clone());
    account(&mut execution, inner, 10000);
    assert_eq!(
        fixture
            .start(&record, execution, true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let first = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    let record = fixture.record("fresh after large state").await;
    let inner = fixture.transport(&record, &url);
    let mut execution = fixture.execution(&record, inner.clone());
    account(&mut execution, inner, 2000);
    assert_eq!(
        fixture
            .start(&record, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let replacement = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replacement.previous_window_id, Some(first.window_id));
    assert_eq!(replacement.phase, CompactionPhase::PreTurn);
    let sent = peer.await.unwrap();
    assert!(sent[1]["input"].as_array().unwrap().contains(&large));
    assert!(!sent[1].to_string().contains("fresh after large state"));
    assert!(sent[2]["input"].as_array().unwrap().contains(&opaque()));
    assert!(!sent[2]["input"].as_array().unwrap().contains(&large));
    assert!(sent[2].to_string().contains("fresh after large state"));
}
