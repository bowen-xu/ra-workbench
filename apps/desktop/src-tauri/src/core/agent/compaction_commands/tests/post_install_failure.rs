//! Hook storage recovery and the persisted Compact terminal are separate facts.
use super::*;
use research_agent_core::context::compaction_hooks::*;

struct FailPostStorage;
impl CompactionHooks for FailPostStorage {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async { CompactionHookOutcome::Continue })
    }
    fn prepare_context(
        &self,
        request: CompactionHookRequest,
        result: CompactionHookResult,
    ) -> HookContextPreparation<'_> {
        Box::pin(async move {
            if request.stage == CompactionHookStage::PostCompact {
                Err(HookContextError)
            } else {
                Ok(result)
            }
        })
    }
}

#[tokio::test]
async fn postcompact_context_storage_error_keeps_failed_terminal_and_installed_window() {
    let f = fixture("/compact").await;
    let transport = transport(&f, false);
    let admission = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async {
            Ok((
                transport as Arc<dyn ModelTransport>,
                Some(Arc::new(FailPostStorage) as Arc<dyn CompactionHooks>),
                empty_tools(),
            ))
        },
    )
    .await
    .unwrap();
    // Context storage did not confirm completion, so the task reports recovery.
    // The manual entry separately commits TurnFinished(Failed) before returning
    // that storage error. Cleanup must preserve this already confirmed terminal.
    assert_eq!(wait(admission).await, TaskCompletion::RecoveryRequired);
    let history = f
        .sessions
        .open(ConversationId::from_uuid(f.record.conversation_id))
        .await
        .unwrap();
    let turn = history.turn(f.record.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
    assert!(turn.failure.is_none());
    assert_eq!(history.active_turn(), None);
    assert_eq!(turn.items.len(), 1);
    assert!(turn.items.values().all(|item| matches!(
        item.kind,
        research_agent_core::protocol::ItemKind::ContextCompaction
    ) && item.completed
        && !item.interrupted));
    let installed = f
        .sessions
        .active_context_checkpoint(f.record.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(installed.turn_id, f.record.turn_id);
    let (started, completed): (u64, u64) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select count(*),count(outcome) from agent_compaction_hook_audit where stage='post_compact'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((started, completed), (1, 0));
    let retry = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("same compact must not reinstall after uncertain hook storage") },
    )
    .await
    .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Failed),
            ..
        }
    ));
    assert!(
        f.sessions
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            .unwrap()
            == installed
    );
    let restored = AgentSessionService::from_app_store(&f.store).unwrap();
    assert_eq!(
        restored
            .open(ConversationId::from_uuid(f.record.conversation_id))
            .await
            .unwrap(),
        history
    );
    assert!(
        restored
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            .unwrap()
            == installed
    );
    crate::agent::compaction_commands::test_fixtures::export_service(
        "post-failed",
        &restored,
        f.record.conversation_id,
    )
    .await;
}
