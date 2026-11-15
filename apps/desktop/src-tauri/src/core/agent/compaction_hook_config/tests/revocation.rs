use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Completions(std::sync::Mutex<Vec<CompactionHookOutcome>>);
impl HookHandlerObserver for Completions {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, event: CompactionHookNotification) {
        if let CompactionHookNotification::Completed(outcome) = event {
            self.0.lock().unwrap().push(outcome);
        }
    }
}

struct Authority {
    allowed: AtomicBool,
    checks: AtomicUsize,
    revision: String,
    handler: Uuid,
}
impl HookInvocationAuthority for Authority {
    fn check(
        &self,
        handler: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        assert_eq!(handler, self.handler);
        assert_eq!(revision, self.revision);
        assert_eq!(request.model, model());
        assert_eq!(request.stage, CompactionHookStage::SessionStartCompact);
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.allowed
            .load(Ordering::SeqCst)
            .then_some(())
            .ok_or(HookConfigError)
    }
}

#[tokio::test]
async fn revoked_authority_blocks_the_frozen_plan_before_spawn_and_rechecks_each_invocation() {
    let root = tempfile::tempdir().unwrap();
    let config = parse(&text(root.path().to_str().unwrap())).unwrap();
    let authority = Arc::new(Authority {
        allowed: AtomicBool::new(false),
        checks: AtomicUsize::new(0),
        revision: config.revision().into(),
        handler: config.handlers()[0].id,
    });
    let completions = Arc::new(Completions::default());
    let hooks = config
        .authorize(
            model(),
            |hook, _| {
                Ok(AuthorizedHookCommand {
                    process: hook.candidate()?,
                    transcript_path: None,
                    launcher: None,
                })
            },
            completions.clone(),
            authority.clone(),
        )
        .unwrap();
    // Freezing initial consent neither executes nor substitutes for live consent.
    assert_eq!(authority.checks.load(Ordering::SeqCst), 0);
    let request = CompactionHookRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        stage: CompactionHookStage::SessionStartCompact,
        phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
        model: model(),
    };
    let denied = hooks.run_with_context(request.clone()).await;
    assert_eq!(denied.outcome, CompactionHookOutcome::Continue);
    assert!(denied.additional_contexts.is_empty());
    assert!(!root.path().join("executed").exists());
    authority.allowed.store(true, Ordering::SeqCst);
    assert_eq!(
        hooks
            .run_with_context(request.clone())
            .await
            .additional_contexts,
        ["approved-context"]
    );
    std::fs::remove_file(root.path().join("executed")).unwrap();
    authority.allowed.store(false, Ordering::SeqCst);
    let denied_again = hooks.run_with_context(request).await;
    assert_eq!(denied_again.outcome, CompactionHookOutcome::Continue);
    assert!(denied_again.additional_contexts.is_empty());
    assert!(!root.path().join("executed").exists());
    assert_eq!(authority.checks.load(Ordering::SeqCst), 3);
    assert_eq!(
        *completions.0.lock().unwrap(),
        [
            CompactionHookOutcome::Failed,
            CompactionHookOutcome::Continue,
            CompactionHookOutcome::Failed
        ]
    );
}
