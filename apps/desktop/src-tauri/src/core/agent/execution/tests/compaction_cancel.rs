use super::*;
use std::sync::atomic::AtomicBool;

struct PendingSummaryTransport {
    ordinary: Arc<Transport>,
    entered: tokio::sync::Notify,
    dropped: Arc<AtomicBool>,
}
struct PendingSummaryStream(Arc<AtomicBool>);
impl Drop for PendingSummaryStream {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl ModelStream for PendingSummaryStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(std::future::pending())
    }
}
impl ModelTransport for PendingSummaryTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.ordinary.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            if !request.tools.is_empty() {
                return self.ordinary.start(request).await;
            }
            self.entered.notify_one();
            Ok(Box::new(PendingSummaryStream(self.dropped.clone())) as Box<dyn ModelStream>)
        })
    }
}

#[tokio::test]
async fn cancel_during_compaction_releases_io_and_persists_canceled_turn() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let pending = Arc::new(PendingSummaryTransport {
        ordinary: transport.clone(),
        entered: tokio::sync::Notify::new(),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    execution.transport = token_compaction(pending.clone(), 3000);
    execution.compaction_strategy = Some(text_strategy(pending.clone()));
    execution.max_input_bytes = usize::MAX;
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
    let admission = execution
        .start(
            service.clone(),
            store,
            ConversationId::from_uuid(id),
            second.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        pending.entered.notified(),
    )
    .await
    .unwrap();
    assert!(
        service
            .tasks()
            .cancel_turn(ConversationId::from_uuid(id), owned.turn_id)
            .unwrap()
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    assert!(pending.dropped.load(Ordering::SeqCst));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert_eq!(
        history.turn(owned.turn_id).unwrap().outcome,
        Some(TurnOutcome::Canceled)
    );
    assert_eq!(
        history.turn(first.turn_id).unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
}

#[tokio::test]
async fn canceled_manual_compact_releases_io_without_installing_a_window() {
    let (service, store, transport, tools, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let pending = Arc::new(PendingSummaryTransport {
        ordinary: transport.clone(),
        entered: tokio::sync::Notify::new(),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    execution.transport = pending.clone();
    execution.compaction_strategy = Some(text_strategy(pending.clone()));
    let execution = Arc::new(execution);
    let first = store
        .put(id, Uuid::new_v4(), input("original"))
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
    let mut command = input("command");
    command.text = "/compact".into();
    let command = store.put(id, Uuid::new_v4(), command).await.unwrap();
    let admission = execution
        .start_compaction(
            service.clone(),
            store,
            ConversationId::from_uuid(id),
            command.request_id,
            None,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new compact task expected")
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        pending.entered.notified(),
    )
    .await
    .unwrap();
    assert!(
        service
            .tasks()
            .cancel_turn(ConversationId::from_uuid(id), owned.turn_id)
            .unwrap()
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    assert!(pending.dropped.load(Ordering::SeqCst));
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let compact = history.turn(command.turn_id).unwrap();
    assert_eq!(compact.items.len(), 1);
    let item = compact.items.values().next().unwrap();
    assert_eq!(
        item.kind,
        research_agent_core::protocol::ItemKind::ContextCompaction
    );
    assert!(item.interrupted && !item.completed);
    assert_eq!(compact.outcome, Some(TurnOutcome::Canceled));
    assert_eq!(compact.attempts.len(), 1);
    assert_eq!(
        history.turn(first.turn_id).unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
}
