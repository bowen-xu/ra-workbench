//! Native post-tool-batch coverage of the pinned TokenBudget buffered trigger.
use super::*;
use research_agent_core::context::token_budget::{
    TokenBudgetPolicy, TokenBudgetSettings, reset::TokenBudgetResetStrategy,
};

#[path = "token_budget/guidance.rs"]
mod guidance;
#[path = "token_budget/hooks.rs"]
mod hooks;
#[path = "token_budget/prompts.rs"]
mod prompts;
#[path = "token_budget/storage_diagnostic.rs"]
mod storage_diagnostic;
#[path = "token_budget/tools.rs"]
mod tools;

struct Tokens {
    before: u64,
    after: u64,
}
impl ContextTokenEstimator for Tokens {
    fn estimate(&self, messages: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
        Some(
            if messages.iter().any(|message| {
                matches!(
                    message,
                    ModelMessage::Assistant { .. } | ModelMessage::CanonicalOutput(_)
                )
            }) {
                self.after
            } else {
                self.before
            },
        )
    }
}
struct Buffered {
    inner: Arc<Transport>,
    accounting: Arc<ContextAccounting>,
    policy: Arc<TokenBudgetPolicy>,
}
impl ModelTransport for Buffered {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn context_accounting(&self) -> Option<Arc<ContextAccounting>> {
        Some(self.accounting.clone())
    }
    fn token_budget_policy(&self) -> Option<Arc<TokenBudgetPolicy>> {
        Some(self.policy.clone())
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}
fn buffered(execution: &mut AgentExecution, transport: Arc<Transport>, after: u64, fallback: bool) {
    execution.compaction_strategy = Some(Arc::new(TokenBudgetResetStrategy::new(model()).unwrap()));
    execution.transport = Arc::new(Buffered {
        inner: transport,
        accounting: Arc::new(ContextAccounting {
            model: model(),
            budget: Some(CompactionBudget {
                context_window_tokens: 200,
                effective_percent: 95,
                configured_trigger_tokens: Some(100),
                output_reserve_tokens: 10,
            }),
            unknown_window_trigger_tokens: None,
            estimator: Arc::new(Tokens { before: 40, after }),
        }),
        policy: Arc::new(
            TokenBudgetPolicy::resolve(TokenBudgetSettings {
                auto_compact_fallback_prompt: fallback.then(|| "prepare for reset".into()),
                auto_compact_fallback_buffer_tokens: Some(30),
                ..Default::default()
            })
            .unwrap(),
        ),
    });
}

#[tokio::test]
async fn buffered_reset_runs_only_after_full_tool_batch_at_effective_limit() {
    for (after, fallback, tool, reset) in [
        (120, true, true, false),
        (130, true, true, true),
        (191, true, true, true), // Hard capacity wins before another sample.
        (120, false, true, true),
        (190, true, false, false), // Natural completion does not create a window.
    ] {
        let (service, store, transport, tools, mut execution, id) = setup().await;
        *transport.responses.lock().unwrap() = VecDeque::from(if tool {
            vec![response(true), response(false)]
        } else {
            vec![response(false)]
        });
        buffered(&mut execution, transport.clone(), after, fallback);
        let record = store
            .put(id, Uuid::new_v4(), input("new evidence"))
            .await
            .unwrap();
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
        let window = service.active_context_checkpoint(id).await.unwrap();
        assert_eq!(
            window.is_some(),
            reset,
            "after={after} fallback={fallback} tool={tool}"
        );
        assert_eq!(tools.0.load(Ordering::SeqCst), usize::from(tool));
        {
            let requests = transport.requests.lock().unwrap();
            assert_eq!(requests.len(), if tool { 2 } else { 1 }); // Reset never calls a model.
            if reset {
                assert!(matches!(
                window.unwrap().output,
                research_agent_core::context::compaction_strategy::CompactionOutput::ResetWindow
            ));
                assert!(
                    !requests[1]
                        .messages
                        .iter()
                        .any(|message| matches!(message, ModelMessage::ToolResult { .. }))
                );
            } else if tool {
                assert!(
                    requests[1]
                        .messages
                        .iter()
                        .any(|message| matches!(message, ModelMessage::ToolResult { .. }))
                );
            }
        }
        let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
        assert!(
            history
                .turn(record.turn_id)
                .unwrap()
                .items
                .values()
                .any(|item| matches!(item.kind, ItemKind::ToolCall { .. }))
                == tool
        );
    }
}

#[tokio::test]
async fn buffer_never_spends_hard_capacity_or_changes_unrelated_strategy() {
    let (_, _, transport, _, mut execution, _) = setup().await;
    buffered(&mut execution, transport.clone(), 120, true);
    let small = vec![ModelMessage::User {
        content: vec![ContentPart::Text("input".into())],
        source_item_id: None,
    }];
    assert!(!execution.should_compact_context(&small, &[], None).unwrap());
    let grown = vec![ModelMessage::Assistant {
        phase: None,
        content: vec![],
        tool_calls: vec![],
    }];
    assert!(!execution.should_compact_context(&grown, &[], None).unwrap());
    execution.check_context_window(&grown, &[], None).unwrap();
    // A large buffer cannot defer the hard window's output-reserved limit.
    let policy = Arc::new(
        TokenBudgetPolicy::resolve(TokenBudgetSettings {
            auto_compact_fallback_prompt: Some("prepare".into()),
            auto_compact_fallback_buffer_tokens: Some(10000),
            ..Default::default()
        })
        .unwrap(),
    );
    execution.transport = Arc::new(Buffered {
        inner: transport.clone(),
        policy,
        accounting: Arc::new(ContextAccounting {
            model: model(),
            budget: Some(CompactionBudget {
                context_window_tokens: 200,
                effective_percent: 95,
                configured_trigger_tokens: Some(100),
                output_reserve_tokens: 10,
            }),
            unknown_window_trigger_tokens: None,
            estimator: Arc::new(Tokens {
                before: 190,
                after: 191,
            }),
        }),
    });
    assert!(execution.should_compact_context(&small, &[], None).unwrap());
    assert!(execution.check_context_window(&grown, &[], None).is_err());
    execution.compaction_strategy = Some(text_strategy(transport));
    assert!(matches!(
        execution.should_compact_context(&small, &[], None),
        Err(RunError::Model(ModelError::UnsupportedCapability))
    ));
}
