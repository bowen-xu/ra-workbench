use super::*;
use research_agent_core::{context::compaction_strategy::CompactionPhase, protocol::TaskKind};

#[path = "compaction_hooks.rs"]
mod compaction_hooks;
#[path = "manual_compaction/persistence_barrier.rs"]
mod persistence_barrier;
#[path = "manual_compaction/reset.rs"]
mod reset;
#[path = "session_start.rs"]
mod session_start;
#[path = "manual_compaction/tool_catalog.rs"]
mod tool_catalog;

fn message_text(messages: &[ModelMessage]) -> String {
    messages
        .iter()
        .flat_map(|message| match message {
            ModelMessage::CanonicalOutput(item) => match &item.normalized {
                Some(research_agent_core::model::ModelOutputItem::Message(message)) => {
                    vec![message.text.as_str()]
                }
                _ => vec![],
            },
            ModelMessage::System(text)
            | ModelMessage::Developer(text)
            | ModelMessage::ToolResult { content: text, .. } => {
                vec![text.as_str()]
            }
            ModelMessage::User { content: parts, .. }
            | ModelMessage::Assistant { content: parts, .. } => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    ContentPart::Image { .. } => None,
                })
                .collect(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn compact(
    execution: Arc<AgentExecution>,
    service: Arc<AgentSessionService>,
    store: Arc<SqliteSubmissionStore>,
    id: Uuid,
) -> (
    research_agent_core::submission::SubmissionRecord,
    SubmissionAdmission,
) {
    let mut command = input("command-metadata");
    command.text = "/compact".into();
    command.context[0].text = "command-only source must not enter model history".into();
    let record = store.put(id, Uuid::new_v4(), command).await.unwrap();
    let admission = execution
        .start_compaction(
            service,
            store,
            ConversationId::from_uuid(id),
            record.request_id,
            None,
        )
        .await
        .unwrap();
    (record, admission)
}
async fn completed(admission: SubmissionAdmission) -> TaskCompletion {
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new task expected")
    };
    owned.task.wait().await
}

#[tokio::test]
async fn manual_compact_installs_without_user_message_or_followup_and_replays_once() {
    let (service, store, transport, tools, execution, id) = setup().await;
    let mut responses = Vec::new();
    for (input_tokens, output_tokens) in [(100, 20), (5000, 200), (30, 5)] {
        let mut events = response(false);
        events.insert(
            events.len() - 1,
            ModelStreamEvent::Usage(ModelUsage {
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
            }),
        );
        responses.push(events);
    }
    *transport.responses.lock().unwrap() = VecDeque::from(responses);
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
    assert_eq!(
        service
            .context_usage(id, model())
            .await
            .unwrap()
            .current_tokens,
        Some(120)
    );
    let (record, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(record.turn_id).unwrap();
    assert_eq!(turn.task_kind, TaskKind::Compact);
    assert_eq!(turn.items.len(), 1);
    let item = turn.items.values().next().unwrap();
    assert_eq!(
        item.kind,
        research_agent_core::protocol::ItemKind::ContextCompaction
    );
    assert!(item.completed && !item.interrupted);
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    let window = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(window.phase, CompactionPhase::Manual);
    assert_eq!(window.turn_id, record.turn_id);
    let compact_usage = service.context_usage(id, model()).await.unwrap();
    assert_eq!(compact_usage.window_id, Some(window.window_id));
    assert_eq!(
        compact_usage.status,
        crate::agent::context_usage::WindowUsageStatus::Unknown
    );
    assert!(compact_usage.current_tokens.is_none());
    assert!(compact_usage.observation.is_none());
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    let retry = execution
        .clone()
        .start_compaction(
            service.clone(),
            store.clone(),
            ConversationId::from_uuid(id),
            record.request_id,
            None,
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
    assert!(matches!(
        execution
            .clone()
            .start(
                service.clone(),
                store.clone(),
                ConversationId::from_uuid(id),
                record.request_id
            )
            .await,
        Err(SubmissionRunError::Input(
            research_agent_core::submission::SubmissionError::Conflict
        ))
    ));
    let next = store
        .put(id, Uuid::new_v4(), input("next-source"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, next.request_id).await,
        TaskCompletion::Completed
    );
    let sampled_usage = service.context_usage(id, model()).await.unwrap();
    assert_eq!(sampled_usage.window_id, Some(window.window_id));
    assert_eq!(
        sampled_usage.status,
        crate::agent::context_usage::WindowUsageStatus::Observed
    );
    assert_eq!(sampled_usage.current_tokens, Some(35));
    assert_eq!(
        sampled_usage.observation.unwrap().sample.turn_id,
        next.turn_id
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].tools.is_empty());
    let encoded = message_text(&requests[1].messages);
    assert!(encoded.contains("old-source"));
    assert!(!encoded.contains("/compact"));
    assert!(!encoded.contains("command-only source"));
    let replay = message_text(&requests[2].messages);
    assert!(!replay.contains("command-only source"));
    assert!(!replay.contains("/compact"));
    assert!(replay.contains("Context checkpoint handoff"));
    assert!(replay.contains("next-source"));
    assert_eq!(
        requests[2]
            .messages
            .iter()
            .filter(|message| matches!(message, ModelMessage::System(_)))
            .count(),
        1
    );
}

#[tokio::test]
async fn failed_manual_compact_keeps_the_installed_window_and_original_journal() {
    let (service, store, transport, _, execution, id) = setup().await;
    let mut incomplete = response(false);
    incomplete.pop(); // EOF without Completed is still a failed compaction.
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(false), response(false), incomplete]);
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
    let (_, first) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(first).await, TaskCompletion::Completed);
    let previous = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    let (record, second) = compact(execution, service.clone(), store, id).await;
    assert_eq!(completed(second).await, TaskCompletion::Failed);
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap()
            == previous
    );
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert_eq!(
        history.turn(record.turn_id).unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    let failed = history.turn(record.turn_id).unwrap();
    assert_eq!(failed.items.len(), 1);
    let item = failed.items.values().next().unwrap();
    assert_eq!(
        item.kind,
        research_agent_core::protocol::ItemKind::ContextCompaction
    );
    assert!(item.interrupted && !item.completed);
    assert_eq!(history.turn(record.turn_id).unwrap().attempts.len(), 1);
    assert!(!history.turn(old.turn_id).unwrap().items.is_empty());
}

#[tokio::test]
async fn first_manual_compact_installs_zero_watermark_and_restores_on_next_turn() {
    let (service, store, transport, tools, execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false), response(false)]);
    let execution = Arc::new(execution);
    let (command, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let saved = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.source_watermark, 0);
    assert_eq!(saved.phase, CompactionPhase::Manual);
    assert!(saved.retained.is_empty());
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert_eq!(history.turn_order().len(), 1);
    assert!(
        history
            .turn(command.turn_id)
            .unwrap()
            .items
            .values()
            .all(|item| item.completed)
    );
    let next = store
        .put(id, Uuid::new_v4(), input("first-user-source"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service, store, id, next.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].tools.is_empty());
    assert_eq!(requests[0].messages.len(), 2); // current rules plus synthetic handoff prompt
    assert!(matches!(&requests[0].messages[0], ModelMessage::System(_)));
    assert!(!message_text(&requests[0].messages).contains("command-only source"));
    let replay = message_text(&requests[1].messages);
    assert!(replay.contains("Context checkpoint handoff"));
    assert!(replay.contains("first-user-source"));
    assert!(!replay.contains("/compact"));
    assert!(!replay.contains("command-only source"));
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn confirmed_compact_rejects_changed_history_before_events_or_inference() {
    let (service, store, transport, tools, execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let execution = Arc::new(execution);
    let confirmed = service
        .open(ConversationId::from_uuid(id))
        .await
        .unwrap()
        .seq();
    let mut command = input("command");
    command.text = "/compact".into();
    let command = store.put(id, Uuid::new_v4(), command).await.unwrap();
    let intervening = store
        .put(id, Uuid::new_v4(), input("intervening"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            intervening.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let watermark = service
        .open(ConversationId::from_uuid(id))
        .await
        .unwrap()
        .seq();
    assert!(watermark > confirmed);
    let result = execution
        .start_confirmed_compaction(
            service.clone(),
            store,
            CompactionStart {
                conversation_id: ConversationId::from_uuid(id),
                request_id: command.request_id,
                expected_revision: confirmed,
            },
            None,
        )
        .await;
    assert!(
        matches!(result, Err(SubmissionRunError::HistoryChanged { expected, actual }) if expected == confirmed && actual == watermark)
    );
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert_eq!(history.seq(), watermark);
    assert!(history.turn(command.turn_id).is_none());
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn confirmed_compact_retry_returns_receipt_after_revision_advances() {
    let (service, store, transport, _, execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let execution = Arc::new(execution);
    let mut command = input("command");
    command.text = "/compact".into();
    let command = store.put(id, Uuid::new_v4(), command).await.unwrap();
    let scope = || CompactionStart {
        conversation_id: ConversationId::from_uuid(id),
        request_id: command.request_id,
        expected_revision: 0,
    };
    let first = execution
        .clone()
        .start_confirmed_compaction(service.clone(), store.clone(), scope(), None)
        .await
        .unwrap();
    assert_eq!(completed(first).await, TaskCompletion::Completed);
    assert!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .seq()
            > 0
    );
    let retry = execution
        .start_confirmed_compaction(service, store, scope(), None)
        .await
        .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[path = "manual_compaction/empty_suffix.rs"]
mod empty_suffix;
