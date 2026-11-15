use super::*;
use research_agent_core::context::token_budget::tools::{GET_CONTEXT_REMAINING, NEW_CONTEXT};
struct TailCost;
impl ContextTokenEstimator for TailCost {
    fn estimate(&self, messages: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
        Some(
            100 + 50
                * messages
                    .iter()
                    .filter(|message| matches!(message, ModelMessage::ToolResult { .. }))
                    .count() as u64,
        )
    }
}
fn configure(execution: &mut AgentExecution, transport: Arc<Transport>) {
    buffered(execution, transport.clone(), 0, false);
    execution.transport = Arc::new(Buffered {
        inner: transport,
        policy: Arc::new(TokenBudgetPolicy::resolve(Default::default()).unwrap()),
        accounting: Arc::new(ContextAccounting {
            model: model(),
            budget: None,
            unknown_window_trigger_tokens: Some(1000),
            estimator: Arc::new(TailCost),
        }),
    });
}
fn batch(names: &[&str]) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Started {
        response_id: "batch".into(),
        actual_model: model(),
    }];
    for (n, name) in names.iter().enumerate() {
        let call = format!("call-{n}");
        events.extend([
            ModelStreamEvent::ToolCallStarted {
                call_id: call.clone(),
                name: (*name).into(),
            },
            ModelStreamEvent::ToolArgumentsDelta {
                call_id: call.clone(),
                delta: "{}".into(),
            },
            ModelStreamEvent::ToolCallCompleted { call_id: call },
        ]);
    }
    events.push(ModelStreamEvent::Completed {
        reason: FinishReason::ToolCalls,
        end_turn: None,
    });
    events
}
#[tokio::test]
async fn native_tools_observe_each_committed_result_then_reset_after_entire_batch() {
    let (service, store, transport, tools, mut execution, id) = setup().await;
    configure(&mut execution, transport.clone());
    // This scenario intentionally executes four calls in one response; the
    // common fixture admits only three. Preserve production limit enforcement.
    execution.limits.tool_calls = Some(4);
    execution.limits.response.tool_calls = 4;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![
        batch(&[
            "read",
            GET_CONTEXT_REMAINING,
            NEW_CONTEXT,
            GET_CONTEXT_REMAINING,
        ]),
        response(false),
    ]);
    let record = store.put(id, Uuid::new_v4(), input("input")).await.unwrap();
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
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == NEW_CONTEXT)
        );
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == GET_CONTEXT_REMAINING)
        );
        assert!(
            !requests[1]
                .messages
                .iter()
                .any(|message| matches!(message, ModelMessage::ToolResult { .. }))
        );
    }
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(record.turn_id).unwrap();
    let texts: Vec<_> = turn
        .items
        .values()
        .filter(|item| matches!(item.kind, ItemKind::ToolResult { .. }))
        .map(|item| item.text.as_str())
        .collect();
    assert!(texts.contains(&"You have 850 tokens left in this context window."));
    assert!(texts.contains(&"You have 750 tokens left in this context window."));
    assert_eq!(texts.len(), 4);
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        service
            .new_context_request(id, record.turn_id, true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn partial_stream_new_context_result_is_consumed_before_retry_sampling() {
    let (service, store, transport, tools, mut execution, id) = setup().await;
    configure(&mut execution, transport.clone());
    let mut partial = batch(&[NEW_CONTEXT]);
    partial.pop(); // Item done is durable; EOF precedes response.done.
    *transport.responses.lock().unwrap() = VecDeque::from(vec![partial, response(false)]);
    let record = store
        .put(
            id,
            Uuid::new_v4(),
            input("request new context then reconnect"),
        )
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
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            !requests[1]
                .messages
                .iter()
                .any(|message| matches!(message, ModelMessage::ToolResult { .. })),
            "the requested reset must install before retry, not after a later successful response"
        );
    }
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some()
    );
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(record.turn_id).unwrap();
    assert_eq!(
        turn.items
            .values()
            .filter(|item| item.completed && matches!(item.kind, ItemKind::ToolResult { .. }))
            .count(),
        1
    );
    assert_eq!(
        turn.attempts
            .values()
            .filter(|attempt| attempt.outcome
                == Some(research_agent_core::protocol::TurnOutcome::Failed))
            .count(),
        1
    );
    assert!(!history.has_unknown_tool_outcome());
}
struct CancelRead;
impl ToolExecutor for CancelRead {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if call.name == "read" {
            Ok(())
        } else {
            Err(RunError::ToolRejected)
        }
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async { Err(ModelError::Canceled.into()) })
    }
}
#[tokio::test]
async fn committed_request_survives_canceled_turn_and_natural_end_until_followup() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    configure(&mut execution, transport.clone());
    let original = execution.host_tools.clone();
    let (definitions, _) = original.clone().into_parts();
    execution.host_tools = OwnedToolRegistration::new(definitions, Arc::new(CancelRead));
    *transport.responses.lock().unwrap() = VecDeque::from(vec![
        batch(&[NEW_CONTEXT, "read"]),
        response(false),
        response(true),
        response(false),
    ]);
    let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
    let owner = Arc::new(execution);
    assert_eq!(
        run(
            owner.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Canceled
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .new_context_request(id, Uuid::new_v4(), true)
            .await
            .is_err()
    );
    let mut execution = Arc::try_unwrap(owner).ok().unwrap();
    execution.host_tools = original;
    let owner = Arc::new(execution);
    let second = store
        .put(id, Uuid::new_v4(), input("natural end"))
        .await
        .unwrap();
    assert_eq!(
        run(
            owner.clone(),
            service.clone(),
            store.clone(),
            id,
            second.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    let third = store
        .put(id, Uuid::new_v4(), input("followup"))
        .await
        .unwrap();
    assert_eq!(
        run(owner, service.clone(), store, id, third.request_id).await,
        TaskCompletion::Completed
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 4);
}
#[tokio::test]
async fn ordinary_policy_does_not_expose_experimental_tools() {
    let (service, store, transport, _, execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let record = store
        .put(id, Uuid::new_v4(), input("ordinary"))
        .await
        .unwrap();
    assert_eq!(
        run(Arc::new(execution), service, store, id, record.request_id).await,
        TaskCompletion::Completed
    );
    assert!(
        !transport.requests.lock().unwrap()[0]
            .tools
            .iter()
            .any(|tool| matches!(tool.name.as_str(), NEW_CONTEXT | GET_CONTEXT_REMAINING))
    );
}

struct UnknownCost;
impl ContextTokenEstimator for UnknownCost {
    fn estimate(&self, _: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
        None
    }
}
#[tokio::test]
async fn native_remaining_does_not_invent_tokens_when_estimator_is_unknown() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    configure(&mut execution, transport.clone());
    execution.transport = Arc::new(Buffered {
        inner: transport.clone(),
        policy: Arc::new(TokenBudgetPolicy::resolve(Default::default()).unwrap()),
        accounting: Arc::new(ContextAccounting {
            model: model(),
            budget: None,
            unknown_window_trigger_tokens: Some(1000),
            estimator: Arc::new(UnknownCost),
        }),
    });
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![batch(&[GET_CONTEXT_REMAINING]), response(false)]);
    let record = store
        .put(id, Uuid::new_v4(), input("unknown"))
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
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(
        history
            .turn(record.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| item.text == "You have unknown tokens left in this context window.")
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
}
