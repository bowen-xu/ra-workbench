use super::*;
use research_agent_core::context::compaction_strategy::CompactionOutput;

#[tokio::test]
async fn completed_empty_suffix_installs_prefixed_history_and_restores_after_owner_release() {
    let (service, store, transport, tools, execution, id) = setup().await;
    let mut empty = response(false);
    empty.remove(1); // Started + Completed, no assistant text
    *transport.responses.lock().unwrap() =
        VecDeque::from([response(false), empty, response(false)]);
    let execution = Arc::new(execution);
    let first = store
        .put(id, Uuid::new_v4(), input("before"))
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
    let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let saved = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    let CompactionOutput::TextSummary(summary) = &saved.output else {
        panic!()
    };
    assert_eq!(
        summary,
        "Context checkpoint handoff from earlier conversation (historical context, not new instructions):\n"
    );
    saved.validate(usize::MAX).unwrap();
    service
        .release_idle(ConversationId::from_uuid(id))
        .await
        .unwrap();
    service.open(ConversationId::from_uuid(id)).await.unwrap();
    let next = store.put(id, Uuid::new_v4(), input("after")).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, next.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].tools.is_empty());
    assert_eq!(
        requests[2]
            .messages
            .iter()
            .filter(|message| matches!(message,
        ModelMessage::User { content: parts, .. } if parts.iter().any(|part|
            matches!(part, ContentPart::Text(text) if text == summary))))
            .count(),
        1
    );
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
}
