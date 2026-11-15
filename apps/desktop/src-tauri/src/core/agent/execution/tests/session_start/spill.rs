use super::*;
use crate::agent::compaction_hook_spill::{SpillingCompactionHooks, file::FileHookSpillStore};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TextHook {
    text: String,
    calls: AtomicUsize,
}
impl CompactionHooks for TextHook {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        unreachable!()
    }
    fn run_with_context(&self, _: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            CompactionHookResult {
                outcome: CompactionHookOutcome::Stop,
                additional_contexts: vec![self.text.clone()],
            }
        })
    }
}
struct Observer;
impl HookHandlerObserver for Observer {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, _: CompactionHookNotification) {}
}

async fn verify(fail: bool) {
    let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
    let root = tempfile::tempdir().unwrap();
    let directory = if fail {
        root.path().join("missing")
    } else {
        root.path().canonicalize().unwrap()
    };
    let text = "whole hook body 中文🙂 ".repeat(1000);
    let hook = Arc::new(TextHook {
        text: text.clone(),
        calls: AtomicUsize::new(0),
    });
    let handlers = ConcurrentCompactionHooks::new(
        vec![CompactionHookHandler {
            id: Uuid::from_u128(10),
            stage: CompactionHookStage::SessionStartCompact,
            hooks: hook.clone(),
        }],
        Arc::new(Observer),
    )
    .unwrap();
    execution.compaction_hooks = Some(Arc::new(
        SpillingCompactionHooks::new(
            Arc::new(handlers),
            Arc::new(FileHookSpillStore::new(directory).unwrap()),
            execution.model.clone(),
            512,
        )
        .unwrap(),
    ));
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(false), response(false), response(false)]);
    let execution = Arc::new(execution);
    let initial = store
        .put(id, Uuid::new_v4(), input("initial"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            initial.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let stopped = store
        .put(id, Uuid::new_v4(), input("stopped"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            stopped.request_id
        )
        .await,
        if fail {
            TaskCompletion::RecoveryRequired
        } else {
            TaskCompletion::Canceled
        }
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    let count: u64 = db
        .lock()
        .unwrap()
        .query_row(
            "select count(*) from agent_compact_hook_contexts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if fail {
        assert_eq!(count, 0);
        assert_eq!(db.lock().unwrap().query_row("select count(*) from agent_compaction_hook_audit where stage='session_start_compact' and notification='completed'", [], |row| row.get::<_,u64>(0)).unwrap(), 0);
        return;
    }
    assert_eq!(count, 1);
    let saved: String = db
        .lock()
        .unwrap()
        .query_row(
            "select contexts_json from agent_compact_hook_contexts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let contexts: Vec<String> = serde_json::from_str(&saved).unwrap();
    assert!(contexts[0].len() <= 512);
    let path = contexts[0]
        .lines()
        .find_map(|line| line.strip_prefix("Full hook output saved to: "))
        .unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    let mut fresh = input("fresh");
    fresh.text = "fresh spill input".into();
    let fresh = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, fresh.request_id).await,
        TaskCompletion::Completed
    );
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    let requests = transport.requests.lock().unwrap();
    let messages = &requests.last().unwrap().messages;
    assert_eq!(
        messages
            .iter()
            .filter(
                |message| matches!(message, ModelMessage::Developer(body) if body == &contexts[0])
            )
            .count(),
        1
    );
    assert!(!message_text(messages).contains(&text));
}

#[tokio::test]
async fn native_spilled_hook_stop_persists_preview_and_restores_without_reexecution() {
    verify(false).await;
}

#[tokio::test]
async fn native_spill_failure_preserves_started_evidence_and_blocks_sampling() {
    verify(true).await;
}
