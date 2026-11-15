use super::*;
use research_agent_core::context::{
    compaction_hooks::*, compaction_strategy::CompactionOutput,
    token_budget::reset::TokenBudgetResetStrategy,
};

struct Stop(CompactionHookStage);
impl CompactionHooks for Stop {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}

    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async move {
            if request.stage == self.0 {
                CompactionHookOutcome::Stop
            } else {
                CompactionHookOutcome::Continue
            }
        })
    }
}

#[tokio::test]
async fn manual_reset_replaces_an_incompatible_summary_without_reading_it() {
    let (service, store, transport, _, execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let owner = Arc::new(execution);
    let (_, admission) = compact(owner.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let previous = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(previous.output, CompactionOutput::TextSummary(_)));
    let mut execution = Arc::try_unwrap(owner).ok().unwrap();
    execution.compaction_strategy = Some(Arc::new(TokenBudgetResetStrategy::new(model()).unwrap()));
    let (_, admission) = compact(Arc::new(execution), service.clone(), store, id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let reset = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reset.previous_window_id, Some(previous.window_id));
    assert!(matches!(reset.output, CompactionOutput::ResetWindow));
    assert!(reset.retained.is_empty());
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reset_manual_task_has_no_model_attempt_or_command_text_and_next_turn_uses_new_window() {
    let (service, store, transport, tools, mut execution, id) = setup().await;
    execution.compaction_strategy = Some(Arc::new(TokenBudgetResetStrategy::new(model()).unwrap()));
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false), response(false)]);
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
    let (record, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(record.turn_id).unwrap();
    assert!(turn.attempts.is_empty());
    assert_eq!(turn.items.len(), 1);
    let window = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert!(window.output == CompactionOutput::ResetWindow);
    assert_eq!(window.window_id.get_version_num(), 7);
    assert!(turn.items[&window.window_id].completed);
    assert!(matches!(
        turn.items[&window.window_id].kind,
        ItemKind::ContextCompaction
    ));
    assert!(window.retained.is_empty());
    assert!(
        window
            .canonical_context
            .as_ref()
            .unwrap()
            .messages
            .is_empty()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    let next = store
        .put(id, Uuid::new_v4(), input("new-source"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution,
            service.clone(),
            store.clone(),
            id,
            next.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let text = {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        message_text(&requests[1].messages)
    };
    assert!(text.contains("new-source"));
    assert!(!text.contains("old-source"));
    assert!(!text.contains("Context checkpoint handoff"));
    assert!(!text.contains("command-only"));
    assert!(!text.contains("/compact"));
    assert_eq!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn(old.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Completed)
    );
}

#[tokio::test]
async fn reset_hook_stops_preserve_install_boundary_without_inference() {
    for stage in [
        CompactionHookStage::PreCompact,
        CompactionHookStage::PostCompact,
    ] {
        let (service, store, transport, _, mut execution, id) = setup().await;
        execution.compaction_strategy =
            Some(Arc::new(TokenBudgetResetStrategy::new(model()).unwrap()));
        execution.compaction_hooks = Some(Arc::new(Stop(stage)));
        let (record, admission) = compact(Arc::new(execution), service.clone(), store, id).await;
        assert_eq!(completed(admission).await, TaskCompletion::Canceled);
        let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
        let turn = history.turn(record.turn_id).unwrap();
        assert!(turn.attempts.is_empty());
        assert!(transport.requests.lock().unwrap().is_empty());
        let checkpoint = service.active_context_checkpoint(id).await.unwrap();
        assert_eq!(
            checkpoint.is_some(),
            stage == CompactionHookStage::PostCompact
        );
        if let Some(window) = checkpoint {
            assert!(turn.items[&window.window_id].completed);
        } else {
            assert!(turn.items.is_empty());
        }
    }
}

#[tokio::test]
async fn reset_pre_turn_keeps_new_input_separate_from_checkpoint_context() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    execution.compaction_strategy = Some(Arc::new(TokenBudgetResetStrategy::new(model()).unwrap()));
    execution.max_input_bytes = usize::MAX;
    execution.transport = token_compaction(transport.clone(), 3000);
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false), response(false)]);
    let execution = Arc::new(execution);
    let mut old_input = input("old-source");
    old_input.context[0].text = "old evidence ".repeat(1200);
    let old = store.put(id, Uuid::new_v4(), old_input).await.unwrap();
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
    let mut next_input = input("new-source");
    next_input.text = "NEW INPUT".into();
    next_input.context[0].text = "new evidence ".repeat(600);
    let next = store.put(id, Uuid::new_v4(), next_input).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, next.request_id).await,
        TaskCompletion::Completed
    );
    let window = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(window.phase, CompactionPhase::PreTurn);
    let canonical = window.restore_reset_window(&|_| None).unwrap();
    assert!(!message_text(&canonical).contains("NEW INPUT"));
    assert!(message_text(&canonical).contains("new evidence"));
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let text = message_text(&requests[1].messages);
    assert!(text.contains("NEW INPUT"));
    assert!(text.contains("new evidence"));
    assert!(!text.contains("old evidence"));
}

#[tokio::test]
async fn reset_failure_never_fabricates_model_evidence_and_failed_install_interrupts_item() {
    for reject_install in [false, true] {
        let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
        execution.compaction_strategy =
            Some(Arc::new(TokenBudgetResetStrategy::new(model()).unwrap()));
        if reject_install {
            db.lock().unwrap().execute_batch("create trigger reject_reset before insert on agent_context_checkpoints begin select raise(abort, 'synthetic storage failure'); end;").unwrap();
        } else {
            execution.max_input_bytes = 1;
        }
        let (record, admission) = compact(Arc::new(execution), service.clone(), store, id).await;
        assert_eq!(
            completed(admission).await,
            if reject_install {
                TaskCompletion::RecoveryRequired
            } else {
                TaskCompletion::Failed
            }
        );
        let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
        let turn = history.turn(record.turn_id).unwrap();
        assert!(turn.attempts.is_empty());
        assert!(transport.requests.lock().unwrap().is_empty());
        assert!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(turn.items.len(), usize::from(reject_install));
        if reject_install {
            let item = turn.items.values().next().unwrap();
            assert!(!item.completed);
            assert!(item.interrupted);
        }
    }
}
