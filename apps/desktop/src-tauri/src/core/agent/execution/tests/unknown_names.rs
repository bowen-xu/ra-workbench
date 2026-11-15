use super::*;
use research_agent_core::{
    model::{CompletedModelItem, ModelOutputItem, ToolCallIntent},
    protocol::{FailureCategory, MAX_EVENT_NAME_CHARS},
};

#[tokio::test]
async fn oversized_tool_identity_is_a_provider_capacity_failure_before_the_real_event_sink() {
    for (call_id, name) in [
        ("id".into(), "x".repeat(MAX_EVENT_NAME_CHARS + 1)),
        ("i".repeat(MAX_EVENT_NAME_CHARS + 1), "read".into()),
    ] {
        let (service, store, transport, tools, execution, id) = setup().await;
        *transport.responses.lock().unwrap() = vec![vec![
            ModelStreamEvent::Started {
                response_id: "r".into(),
                actual_model: model(),
            },
            ModelStreamEvent::OutputItemDone(Box::new(CompletedModelItem {
                canonical_item_id: None,
                native: None,
                normalized: Some(ModelOutputItem::ToolCall(ToolCallIntent {
                    call_id,
                    name,
                    raw_arguments_json: "{}".into(),
                    namespace: None,
                })),
            })),
            ModelStreamEvent::Completed {
                reason: FinishReason::ToolCalls,
                end_turn: None,
            },
        ]]
        .into();
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
            TaskCompletion::Failed
        );
        let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
        let turn = history.turn(record.turn_id).unwrap();
        let (cause, category) = turn.failure.as_ref().unwrap();
        assert!(
            cause.contains("model response exceeded its budget"),
            "{cause}"
        );
        assert_eq!(*category, FailureCategory::Provider);
        assert_eq!(tools.0.load(Ordering::SeqCst), 0);
        assert!(
            !turn
                .items
                .values()
                .any(|item| matches!(item.kind, ItemKind::ToolCall { .. }))
        );
    }
}
