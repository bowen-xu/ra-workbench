//! Native hook context across a buffered reset after a committed tool batch.
use super::*;
use research_agent_core::context::compaction_hooks::*;

struct ToolBatchTokens;
impl ContextTokenEstimator for ToolBatchTokens {
    fn estimate(&self, messages: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
        // Only the committed tool batch exhausts this fixture's window. The
        // small answer produced after reset must survive into the next turn.
        Some(
            if messages
                .iter()
                .any(|message| matches!(message, ModelMessage::ToolResult { .. }))
            {
                130
            } else {
                40
            },
        )
    }
}

struct Hooks {
    stop: bool,
    starts: AtomicUsize,
}
impl CompactionHooks for Hooks {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async { CompactionHookOutcome::Continue })
    }
    fn run_with_context(&self, request: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        Box::pin(async move {
            if request.stage != CompactionHookStage::SessionStartCompact {
                return CompactionHookResult::without_context(CompactionHookOutcome::Continue);
            }
            assert_eq!(
                request.phase,
                research_agent_core::context::compaction_strategy::CompactionPhase::MidTurn
            );
            self.starts.fetch_add(1, Ordering::SeqCst);
            CompactionHookResult {
                outcome: if self.stop {
                    CompactionHookOutcome::Stop
                } else {
                    CompactionHookOutcome::Continue
                },
                additional_contexts: vec!["reset hook context".into()],
            }
        })
    }
}

#[tokio::test]
async fn reset_hook_context_survives_stop_and_continues_after_the_tool_batch() {
    for stop in [false, true] {
        let (service, store, transport, tools, mut execution, id) = setup().await;
        *transport.responses.lock().unwrap() =
            VecDeque::from(vec![response(true), response(false), response(false)]);
        buffered(&mut execution, transport.clone(), 130, true);
        let accounting = execution.transport.context_accounting().unwrap();
        execution.transport = Arc::new(Buffered {
            inner: transport.clone(),
            accounting: Arc::new(ContextAccounting {
                model: accounting.model.clone(),
                budget: accounting.budget,
                unknown_window_trigger_tokens: accounting.unknown_window_trigger_tokens,
                estimator: Arc::new(ToolBatchTokens),
            }),
            policy: execution.transport.token_budget_policy().unwrap(),
        });
        let hooks = Arc::new(Hooks {
            stop,
            starts: AtomicUsize::new(0),
        });
        execution.compaction_hooks = Some(hooks.clone());
        let execution = Arc::new(execution);
        let first = store
            .put(id, Uuid::new_v4(), input("reset original evidence"))
            .await
            .unwrap();
        assert_eq!(
            run(
                execution.clone(),
                service.clone(),
                store.clone(),
                id,
                first.request_id
            )
            .await,
            if stop {
                TaskCompletion::Canceled
            } else {
                TaskCompletion::Completed
            }
        );
        let window = service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            window.output
                == research_agent_core::context::compaction_strategy::CompactionOutput::ResetWindow
        );
        assert_eq!(tools.0.load(Ordering::SeqCst), 1);
        {
            let requests = transport.requests.lock().unwrap();
            assert_eq!(
                requests.len(),
                if stop { 1 } else { 2 },
                "reset has no summary sampling"
            );
            assert_eq!(occurrences(&requests[0].messages), 0);
            if !stop {
                assert_eq!(occurrences(&requests[1].messages), 1);
            }
        }
        let mut next_input = input("reset next evidence");
        next_input.text = "continue after reset hook".into();
        let next = store.put(id, Uuid::new_v4(), next_input).await.unwrap();
        assert_eq!(
            run(execution, service.clone(), store, id, next.request_id).await,
            TaskCompletion::Completed
        );
        assert_eq!(
            hooks.starts.load(Ordering::SeqCst),
            1,
            "resume must not re-execute hook"
        );
        assert_eq!(
            tools.0.load(Ordering::SeqCst),
            1,
            "historical tools stay committed"
        );
        assert_eq!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .unwrap()
                .window_id,
            window.window_id
        );
        let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
        assert_eq!(
            history
                .turn(first.turn_id)
                .unwrap()
                .items
                .values()
                .filter(|item| item.completed && matches!(item.kind, ItemKind::ToolResult { .. }))
                .count(),
            1
        );
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), if stop { 2 } else { 3 });
        let messages = &requests.last().unwrap().messages;
        assert_eq!(occurrences(messages), 1);
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, ModelMessage::ToolResult { .. })),
            "reset does not restore discarded model history"
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| matches!(message,
                    ModelMessage::CanonicalOutput(item)
                    if matches!(&item.normalized, Some(ModelOutputItem::Message(message)) if message.text == "answer")
                ))
                .count(),
            usize::from(!stop),
            "only the answer sampled after reset survives into the next turn"
        );
        let hook = messages.iter().position(|message| matches!(message,ModelMessage::Developer(body) if body == "reset hook context")).unwrap();
        let input = messages.iter().position(|message| matches!(message,ModelMessage::User { content: parts, .. } if parts == &vec![ContentPart::Text("continue after reset hook".into())])).unwrap();
        assert!(hook < input);
    }
}

fn occurrences(messages: &[ModelMessage]) -> usize {
    messages.iter().filter(|message| matches!(message,ModelMessage::Developer(body) if body == "reset hook context")).count()
}
