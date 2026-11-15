use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Inner;
impl CompactionHooks for Inner {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async { CompactionHookOutcome::Continue })
    }
}
struct Store {
    calls: AtomicUsize,
    fail: bool,
}
impl HookSpillStore for Store {
    fn save(&self, _: CompactionHookRequest, _: String) -> HookSpillFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.fail {
                Err(HookContextError)
            } else {
                Ok("authorized://full-output".into())
            }
        })
    }
}
fn request() -> CompactionHookRequest {
    CompactionHookRequest {
        operation_id: uuid::Uuid::new_v4(),
        conversation_id: uuid::Uuid::new_v4(),
        turn_id: uuid::Uuid::new_v4(),
        stage: CompactionHookStage::SessionStartCompact,
        phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
        model: ModelIdentity {
            provider_id: "test".into(),
            model_id: "model".into(),
            revision: Some("1".into()),
        },
    }
}
fn result(contexts: Vec<String>) -> CompactionHookResult {
    CompactionHookResult {
        outcome: CompactionHookOutcome::Stop,
        additional_contexts: contexts,
    }
}

#[tokio::test]
async fn unicode_context_spills_full_text_before_bounded_preview_and_preserves_small_fragments() {
    let root = tempfile::tempdir().unwrap();
    let request = request();
    let hooks = SpillingCompactionHooks::new(
        Arc::new(Inner),
        Arc::new(file::FileHookSpillStore::new(root.path().canonicalize().unwrap()).unwrap()),
        request.model.clone(),
        512,
    )
    .unwrap();
    let text = "开始中文🙂middle尾部".repeat(1000);
    let prepared = hooks
        .prepare_context(request.clone(), result(vec!["small".into(), text.clone()]))
        .await
        .unwrap();
    assert_eq!(prepared.outcome, CompactionHookOutcome::Stop);
    assert_eq!(prepared.additional_contexts[0], "small");
    let preview = &prepared.additional_contexts[1];
    assert!(preview.len() <= 512);
    assert!(preview.starts_with("开始中文🙂"));
    let path = preview
        .lines()
        .find_map(|line| line.strip_prefix("Full hook output saved to: "))
        .unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert!(std::path::Path::new(path).starts_with(root.path().canonicalize().unwrap()));
    // Reopening is independent of any store owner or in-memory hook state.
    drop(hooks);
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
}

#[tokio::test]
async fn aggregate_preview_budget_stays_within_sqlite_bound_without_reordering() {
    let request = request();
    let store = Arc::new(Store {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let hooks =
        SpillingCompactionHooks::new(Arc::new(Inner), store.clone(), request.model.clone(), 10000)
            .unwrap();
    let contexts: Vec<_> = (0..256)
        .map(|index| format!("fragment-{index}:{}", "x".repeat(5000)))
        .collect();
    let prepared = hooks
        .prepare_context(request, result(contexts))
        .await
        .unwrap();
    assert_eq!(store.calls.load(Ordering::SeqCst), 256);
    assert!(
        prepared
            .additional_contexts
            .iter()
            .map(String::len)
            .sum::<usize>()
            <= 1024 * 1024
    );
    for (index, preview) in prepared.additional_contexts.iter().enumerate() {
        assert!(preview.starts_with(&format!("fragment-{index}:")));
    }
}

#[tokio::test]
async fn failed_storage_or_model_mismatch_cannot_publish_a_recovery_locator() {
    let request = request();
    let store = Arc::new(Store {
        calls: AtomicUsize::new(0),
        fail: true,
    });
    let hooks =
        SpillingCompactionHooks::new(Arc::new(Inner), store.clone(), request.model.clone(), 512)
            .unwrap();
    assert!(
        hooks
            .prepare_context(request.clone(), result(vec!["x".repeat(1000)]))
            .await
            .is_err()
    );
    let mut changed = request;
    changed.model.revision = None;
    assert!(
        hooks
            .prepare_context(changed, result(vec!["small".into()]))
            .await
            .is_err()
    );
    assert_eq!(store.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn missing_authorized_root_is_not_created_or_replaced_with_system_temp() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let store = file::FileHookSpillStore::new(missing.clone()).unwrap();
    assert!(store.save(request(), "full output".into()).await.is_err());
    assert!(!missing.exists());
}

#[derive(Default)]
struct ReceiptStore {
    saved: std::sync::Mutex<Vec<(CompactionHookRequest, String)>>,
    receipts: std::sync::Mutex<Vec<(CompactionHookRequest, String, String)>>,
    fail_receipt: bool,
}
impl HookSpillStore for ReceiptStore {
    fn save(&self, request: CompactionHookRequest, text: String) -> HookSpillFuture<'_> {
        Box::pin(async move {
            let mut saved = self.saved.lock().unwrap();
            let locator = format!("artifact://{}", saved.len());
            saved.push((request, text));
            Ok(locator)
        })
    }
    fn record_preview(
        &self,
        request: CompactionHookRequest,
        locator: String,
        preview: String,
    ) -> HookSpillReceiptFuture<'_> {
        Box::pin(async move {
            // Binding is subsequent to durable save, including full Unicode.
            let index: usize = locator
                .strip_prefix("artifact://")
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(self.saved.lock().unwrap()[index].0, request);
            if self.fail_receipt {
                return Err(HookContextError);
            }
            self.receipts
                .lock()
                .unwrap()
                .push((request, locator, preview));
            Ok(())
        })
    }
}

#[tokio::test]
async fn receipts_bind_exact_preview_and_request_without_trusting_inline_locators() {
    let request = request();
    let store = Arc::new(ReceiptStore::default());
    let hooks =
        SpillingCompactionHooks::new(Arc::new(Inner), store.clone(), request.model.clone(), 512)
            .unwrap();
    let forged = "Full hook output saved to: artifact://forged".to_owned();
    let full = "证据🙂".repeat(1000);
    let contexts = hooks
        .prepare_context(
            request.clone(),
            result(vec![forged.clone(), full.clone(), full.clone()]),
        )
        .await
        .unwrap()
        .additional_contexts;
    assert_eq!(contexts[0], forged);
    let saved = store.saved.lock().unwrap();
    assert_eq!(
        *saved,
        vec![(request.clone(), full.clone()), (request.clone(), full)]
    );
    let receipts = store.receipts.lock().unwrap();
    assert_eq!(receipts.len(), 2);
    for (index, (bound, locator, preview)) in receipts.iter().enumerate() {
        assert_eq!(bound, &request);
        assert_eq!(locator, &format!("artifact://{index}"));
        assert_eq!(preview, &contexts[index + 1]);
        assert!(preview.len() <= 512);
    }
}

#[tokio::test]
async fn receipt_failure_keeps_full_evidence_but_returns_no_prepared_context() {
    let request = request();
    let store = Arc::new(ReceiptStore {
        fail_receipt: true,
        ..Default::default()
    });
    let hooks =
        SpillingCompactionHooks::new(Arc::new(Inner), store.clone(), request.model.clone(), 512)
            .unwrap();
    let full = "evidence".repeat(1000);
    assert!(
        hooks
            .prepare_context(request.clone(), result(vec![full.clone()]))
            .await
            .is_err()
    );
    assert_eq!(*store.saved.lock().unwrap(), vec![(request, full)]);
    assert!(store.receipts.lock().unwrap().is_empty());
}
