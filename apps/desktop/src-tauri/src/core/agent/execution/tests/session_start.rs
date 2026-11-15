use super::*;
use research_agent_core::context::compaction_hooks::*;
#[cfg(unix)]
#[path = "session_start/process.rs"]
mod process;
#[path = "session_start/resume.rs"]
mod resume;
#[path = "session_start/spill.rs"]
mod spill;
#[path = "session_start/storage_failure.rs"]
mod storage_failure;

struct Hooks {
    stop_at: Option<CompactionHookStage>,
    events: Mutex<Vec<CompactionHookRequest>>,
}
impl CompactionHooks for Hooks {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async move {
            let stage = request.stage;
            self.events.lock().unwrap().push(request);
            if Some(stage) == self.stop_at {
                CompactionHookOutcome::Stop
            } else {
                CompactionHookOutcome::Continue
            }
        })
    }
    fn run_with_context(&self, request: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        Box::pin(async move {
            let stage = request.stage;
            let outcome = self.run(request).await;
            CompactionHookResult {
                outcome,
                additional_contexts: if stage == CompactionHookStage::SessionStartCompact {
                    vec!["persist before stopping".into()]
                } else {
                    vec![]
                },
            }
        })
    }
}

async fn verify(stop_at: CompactionHookStage) {
    let ((service, store, transport, _, execution, id), db) = setup_with_connection().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![
        response(false),
        response(false),
        response(false),
        response(false),
    ]);
    let execution = Arc::new(execution);
    let old = store
        .put(id, Uuid::new_v4(), input("old-source"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            old.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let mut execution = Arc::try_unwrap(execution).ok().unwrap();
    let hooks = Arc::new(Hooks {
        stop_at: Some(stop_at),
        events: Mutex::new(vec![]),
    });
    execution.compaction_hooks = Some(hooks.clone());
    let execution = Arc::new(execution);
    let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    let post_stop = stop_at == CompactionHookStage::PostCompact;
    assert_eq!(
        completed(admission).await,
        if post_stop {
            TaskCompletion::Canceled
        } else {
            TaskCompletion::Completed
        }
    );
    assert_eq!(
        hooks.events.lock().unwrap().len(),
        2,
        "manual compact queues but does not execute SessionStart"
    );
    let window = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    let next = store
        .put(id, Uuid::new_v4(), input("next-source"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            next.request_id
        )
        .await,
        if post_stop {
            TaskCompletion::Completed
        } else {
            TaskCompletion::Canceled
        }
    );
    assert_eq!(
        transport.requests.lock().unwrap().len(),
        if post_stop { 3 } else { 2 }
    );
    let hook = hooks.events.lock().unwrap()[2].clone();
    assert_eq!(hook.stage, CompactionHookStage::SessionStartCompact);
    let contexts: String = db
        .lock()
        .unwrap()
        .query_row(
            "select contexts_json from agent_compact_hook_contexts where window_id=?1",
            [window.window_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&contexts).unwrap(),
        ["persist before stopping"]
    );
    assert_eq!(hook.operation_id, window.window_id);
    assert_eq!(hook.turn_id, next.turn_id);
    let status: String = db.lock().unwrap().query_row("select outcome from agent_compaction_hook_audit where stage='session_start_compact' and notification='completed'",[],|r|r.get(0)).unwrap();
    assert_eq!(status, if post_stop { "continue" } else { "stop" });

    assert_eq!(hook.phase, CompactionPhase::Manual);
    assert_eq!(hook.model, model());
    let final_input = store
        .put(id, Uuid::new_v4(), input("last-source"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution,
            service.clone(),
            store,
            id,
            final_input.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert_eq!(
        hooks.events.lock().unwrap().len(),
        3,
        "stopped SessionStart is consumed once"
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
    assert!(
        message_text(&transport.requests.lock().unwrap().last().unwrap().messages)
            .contains("Context checkpoint handoff")
    );
    let requests = transport.requests.lock().unwrap();
    let messages = &requests.last().unwrap().messages;
    let contexts: Vec<_> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            matches!(message, ModelMessage::Developer(body) if body == "persist before stopping")
                .then_some(index)
        })
        .collect();
    assert_eq!(contexts.len(), 1, "saved hook body is restored once");
    let next_input = messages.iter().rposition(|message| {
        matches!(message, ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text == &final_input.input.text)))
    }).unwrap();
    assert!(contexts[0] < next_input);
}

#[tokio::test]
async fn session_start_stop_preserves_window_and_does_not_replay_on_next_turn() {
    verify(CompactionHookStage::SessionStartCompact).await;
}

#[tokio::test]
async fn post_stop_leaves_session_start_pending_for_next_regular_turn() {
    verify(CompactionHookStage::PostCompact).await;
}

#[tokio::test]
async fn two_manual_compactions_dispatch_both_pending_hooks_in_order() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = (0..4).map(|_| response(false)).collect();
    let hooks = Arc::new(Hooks {
        stop_at: None,
        events: Mutex::new(vec![]),
    });
    execution.compaction_hooks = Some(hooks.clone());
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
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
    for _ in 0..2 {
        let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
        assert_eq!(completed(admission).await, TaskCompletion::Completed);
    }
    let next = store.put(id, Uuid::new_v4(), input("next")).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, next.request_id).await,
        TaskCompletion::Completed
    );
    let events = hooks.events.lock().unwrap();
    assert_eq!(
        events.iter().map(|event| event.stage).collect::<Vec<_>>(),
        vec![
            CompactionHookStage::PreCompact,
            CompactionHookStage::PostCompact,
            CompactionHookStage::PreCompact,
            CompactionHookStage::PostCompact,
            CompactionHookStage::SessionStartCompact,
            CompactionHookStage::SessionStartCompact,
        ]
    );
    assert_eq!(events[4].operation_id, events[0].operation_id);
    assert_eq!(events[5].operation_id, events[2].operation_id);
    assert!(events[4].operation_id != events[5].operation_id);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.last().unwrap().messages.iter().filter(|message| matches!(message, ModelMessage::Developer(body) if body == "persist before stopping")).count(), 2);
}

#[tokio::test]
async fn pending_manual_hook_waits_until_pre_turn_compaction_finishes() {
    use research_agent_core::context::{
        accounting::{ContextAccounting, ContextTokenEstimator},
        compaction_strategy::CompactionBudget,
    };
    struct FixedUsage;
    impl ContextTokenEstimator for FixedUsage {
        fn estimate(&self, _: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
            Some(700)
        }
    }
    struct TriggerTransport(Arc<Transport>);
    impl ModelTransport for TriggerTransport {
        fn capabilities(&self) -> ModelCapabilities {
            self.0.capabilities()
        }
        fn context_accounting(&self) -> Option<Arc<ContextAccounting>> {
            Some(Arc::new(ContextAccounting {
                unknown_window_trigger_tokens: None,
                model: model(),
                estimator: Arc::new(FixedUsage),
                budget: Some(CompactionBudget {
                    context_window_tokens: 10_000,
                    effective_percent: 95,
                    configured_trigger_tokens: Some(600),
                    output_reserve_tokens: 100,
                }),
            }))
        }
        fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
            self.0.start(request)
        }
    }
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = (0..4).map(|_| response(false)).collect();
    execution.max_input_bytes = usize::MAX;
    execution.transport = Arc::new(TriggerTransport(transport.clone()));
    let hooks = Arc::new(Hooks {
        stop_at: None,
        events: Mutex::new(vec![]),
    });
    execution.compaction_hooks = Some(hooks.clone());
    let execution = Arc::new(execution);
    let mut first = input("first");
    first.context[0].text = "first evidence ".repeat(500);
    let first = store.put(id, Uuid::new_v4(), first).await.unwrap();
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
    let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let mut fresh = input("fresh");
    fresh.context[0].text = "second evidence ".repeat(500);
    let fresh = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, fresh.request_id).await,
        TaskCompletion::Completed
    );
    let events = hooks.events.lock().unwrap();
    assert_eq!(
        events.iter().map(|event| event.stage).collect::<Vec<_>>(),
        vec![
            CompactionHookStage::PreCompact,
            CompactionHookStage::PostCompact,
            CompactionHookStage::PreCompact,
            CompactionHookStage::PostCompact,
            CompactionHookStage::SessionStartCompact,
            CompactionHookStage::SessionStartCompact,
        ]
    );
    assert_eq!(events[4].operation_id, events[0].operation_id);
    assert_eq!(events[5].operation_id, events[2].operation_id);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[2].tools.is_empty());
    assert!(!message_text(&requests[2].messages).contains("persist before stopping"));
    assert_eq!(requests[3].messages.iter().filter(|message| matches!(message, ModelMessage::Developer(body) if body == "persist before stopping")).count(), 2);
}

#[tokio::test]
async fn automatic_compaction_runs_session_start_before_resuming_sampling() {
    verify_automatic(true).await;
}

#[tokio::test]
async fn automatic_session_start_context_enters_the_resumed_model_request() {
    verify_automatic(false).await;
}

async fn verify_automatic(stop: bool) {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(false), response(false), response(false)]);
    execution.max_input_bytes = usize::MAX;
    execution.transport = token_compaction(transport.clone(), 3000);
    let hooks = Arc::new(Hooks {
        stop_at: stop.then_some(CompactionHookStage::SessionStartCompact),
        events: Mutex::new(vec![]),
    });
    execution.compaction_hooks = Some(hooks.clone());
    let execution = Arc::new(execution);
    let mut first = input("v1");
    first.context[0].text = "first evidence ".repeat(1000);
    let first = store.put(id, Uuid::new_v4(), first).await.unwrap();
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
    let mut second = input("v2");
    second.context[0].text = "second evidence ".repeat(500);
    let second = store.put(id, Uuid::new_v4(), second).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, second.request_id).await,
        if stop {
            TaskCompletion::Canceled
        } else {
            TaskCompletion::Completed
        }
    );
    assert_eq!(
        transport.requests.lock().unwrap().len(),
        if stop { 2 } else { 3 },
        "summary only; next sampling was stopped"
    );
    if !stop {
        let requests = transport.requests.lock().unwrap();
        let bodies: Vec<_> = requests
            .last()
            .unwrap()
            .messages
            .iter()
            .filter_map(|message| {
                if let ModelMessage::Developer(body) = message {
                    Some(body.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            bodies
                .iter()
                .filter(|body| **body == "persist before stopping")
                .count(),
            1
        );
        let messages = &requests.last().unwrap().messages;
        let hook = messages
            .iter()
            .position(|message| {
                matches!(message,
            ModelMessage::Developer(body) if body == "persist before stopping")
            })
            .unwrap();
        let input = messages
            .iter()
            .rposition(|message| {
                matches!(message,
            ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part,
                ContentPart::Text(text) if text == &second.input.text)))
            })
            .unwrap();
        assert!(hook < input, "pre-turn hook precedes fresh user input");
    }
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        service
            .take_compact_session_start(id)
            .await
            .unwrap()
            .is_none()
    );
    let events = hooks.events.lock().unwrap();
    assert_eq!(
        events.iter().map(|event| event.stage).collect::<Vec<_>>(),
        vec![
            CompactionHookStage::PreCompact,
            CompactionHookStage::PostCompact,
            CompactionHookStage::SessionStartCompact
        ]
    );
    assert_eq!(events[2].phase, CompactionPhase::PreTurn);
    assert_eq!(events[2].turn_id, second.turn_id);
}
