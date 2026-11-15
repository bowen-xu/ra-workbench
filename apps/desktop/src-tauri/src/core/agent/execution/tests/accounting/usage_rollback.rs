//! Rollback invalidates observations even when the same compacted window survives.
use super::*;
use crate::agent::{context_usage::WindowUsageStatus, service::ConversationRollbackRequest};

#[tokio::test]
async fn surviving_window_rollback_rejects_old_observation_until_new_sampling() {
    let (service, store, transport, tools, execution, id) = setup().await;
    let mut responses = Vec::new();
    for (input_tokens, output_tokens) in [(100, 20), (5000, 200), (200, 40), (30, 5)] {
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
    *transport.responses.lock().unwrap() = responses.into();
    let execution = Arc::new(execution);
    let first = store
        .put(id, Uuid::new_v4(), input("first-source"))
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
        TaskCompletion::Completed
    );
    let command = store
        .put(id, Uuid::new_v4(), input("/compact"))
        .await
        .unwrap();
    let admission = execution
        .clone()
        .start_compaction(
            service.clone(),
            store.clone(),
            ConversationId::from_uuid(id),
            command.request_id,
            None,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new compact task")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let window = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    let second = store
        .put(id, Uuid::new_v4(), input("second-source"))
        .await
        .unwrap();
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
    let measured = service.context_usage(id, model()).await.unwrap();
    assert_eq!(measured.current_tokens, Some(240));
    let old_sample = measured.observation.unwrap().sample;
    let receipt = service
        .rollback_conversation(ConversationRollbackRequest {
            conversation_id: ConversationId::from_uuid(id),
            request_id: Uuid::new_v4(),
            expected_watermark: measured.watermark,
            expected_last_turn: second.turn_id,
            user_turns: 1,
        })
        .await
        .unwrap();
    assert_eq!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap()
            .window_id,
        window.window_id
    );
    let rolled = service.context_usage(id, model()).await.unwrap();
    assert_eq!(rolled.watermark, receipt.watermark);
    assert_eq!(rolled.rollback_epoch, 1);
    assert_eq!(rolled.window_id, Some(window.window_id));
    assert_eq!(rolled.status, WindowUsageStatus::Unknown);
    assert!(rolled.current_tokens.is_none() && rolled.observation.is_none());
    assert!(service.record_context_usage(id, old_sample).await.is_err());
    let third = store
        .put(id, Uuid::new_v4(), input("third-source"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, third.request_id).await,
        TaskCompletion::Completed
    );
    let fresh = service.context_usage(id, model()).await.unwrap();
    assert_eq!(fresh.status, WindowUsageStatus::Observed);
    assert_eq!(fresh.current_tokens, Some(35));
    assert_eq!(fresh.rollback_epoch, 1);
    assert_eq!(fresh.observation.unwrap().sample.turn_id, third.turn_id);
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    let replay = requests[3]
        .messages
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
            | ModelMessage::ToolResult { content: text, .. } => vec![text.as_str()],
            ModelMessage::User { content: parts, .. }
            | ModelMessage::Assistant { content: parts, .. } => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    ContentPart::Image { .. } => None,
                })
                .collect::<Vec<_>>(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!replay.contains("second-source"));
    assert!(replay.contains("third-source"));
}
