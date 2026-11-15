use super::*;
use crate::agent::composed_tools::ComposedToolExecutor;
use research_agent_core::{
    model::ToolDefinition,
    tool_registry::{ToolRegistration, ToolRegistry},
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;

fn call(name: &str) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "owned-call".into(),
        name: name.into(),
        arguments: serde_json::json!({}),
    }
}

struct OwnedTool {
    entered: Notify,
    release: Notify,
    constructed: AtomicUsize,
    completed: AtomicBool,
    safe_to_drop: bool,
}

impl OwnedTool {
    fn new(safe_to_drop: bool) -> Self {
        Self {
            entered: Notify::new(),
            release: Notify::new(),
            constructed: AtomicUsize::new(0),
            completed: AtomicBool::new(false),
            safe_to_drop,
        }
    }
}

impl ToolExecutor for OwnedTool {
    fn cancellation_safe_to_drop(&self, _: &ToolCall) -> bool {
        self.safe_to_drop
    }
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        self.constructed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            self.entered.notify_one();
            self.release.notified().await;
            self.completed.store(true, Ordering::SeqCst);
            Ok(ToolOutput {
                content: "owner teardown completed".into(),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn cancellation_before_dispatch_does_not_construct_an_inner_operation() {
    let owner = OwnedTool::new(false);
    let cancellation = AgentCancellation::default();
    let wrapper = CancellableTools {
        inner: &owner,
        cancellation: cancellation.clone(),
    };
    let future = wrapper.authorize_and_execute(call("write"));
    cancellation.cancel();
    assert!(matches!(
        future.await,
        Err(RunError::Model(ModelError::Canceled))
    ));
    assert_eq!(owner.constructed.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_keeps_polling_started_owner_until_its_result_is_known() {
    let owner = OwnedTool::new(false);
    let cancellation = AgentCancellation::default();
    let wrapper = CancellableTools {
        inner: &owner,
        cancellation: cancellation.clone(),
    };
    let mut execution = wrapper.authorize_and_execute(call("write"));
    tokio::select! {
        _ = owner.entered.notified() => {},
        _ = &mut execution => panic!("owner must be waiting"),
    }
    cancellation.cancel();
    assert!(
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(execution.as_mut().poll(cx).is_pending())
        })
        .await
    );
    assert!(!owner.completed.load(Ordering::SeqCst));
    owner.release.notify_one();
    let output = execution.await.unwrap();
    assert_eq!(output.content, "owner teardown completed");
    assert!(owner.completed.load(Ordering::SeqCst));
}

struct PendingApproval(OwnedTool);
impl ToolExecutor for PendingApproval {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.0.validate(call)
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        self.0.authorize_and_execute(call)
    }
    fn cancel_pending(&self, _: ToolCall) -> RunFuture<'_, ()> {
        self.0.release.notify_one();
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn cancellation_notifies_pending_approval_then_waits_for_its_owner_result() {
    let owner = PendingApproval(OwnedTool::new(false));
    let cancellation = AgentCancellation::default();
    let wrapper = CancellableTools {
        inner: &owner,
        cancellation: cancellation.clone(),
    };
    let mut execution = wrapper.authorize_and_execute(call("write"));
    tokio::select! {
        _ = owner.0.entered.notified() => {},
        _ = &mut execution => panic!("approval owner must wait"),
    }
    cancellation.cancel();
    let output = tokio::time::timeout(std::time::Duration::from_secs(2), execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.content, "owner teardown completed");
    assert!(owner.0.completed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn ready_tool_result_wins_over_simultaneously_ready_cancellation() {
    // Even a drop-safe tool must retain a result already ready for observation.
    let owner = OwnedTool::new(true);
    let cancellation = AgentCancellation::default();
    let wrapper = CancellableTools {
        inner: &owner,
        cancellation: cancellation.clone(),
    };
    let mut execution = wrapper.authorize_and_execute(call("read"));
    tokio::select! {
        _ = owner.entered.notified() => {},
        _ = &mut execution => panic!("owner must be waiting"),
    }
    owner.release.notify_one();
    cancellation.cancel();
    assert_eq!(execution.await.unwrap().content, "owner teardown completed");
}

struct FinalizationOwner {
    name: &'static str,
    offer: AtomicBool,
    released: AtomicUsize,
    committed: AtomicUsize,
    canceled: AtomicUsize,
}

impl ToolExecutor for FinalizationOwner {
    fn cancel_pending(&self, _: ToolCall) -> RunFuture<'_, ()> {
        self.canceled.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
    fn supports_parallel(&self, call: &ToolCall) -> bool {
        call.name == self.name && self.name == "read"
    }
    fn cancellation_safe_to_drop(&self, call: &ToolCall) -> bool {
        self.supports_parallel(call)
    }
    fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        Box::pin(async {
            Ok(self
                .offer
                .swap(false, Ordering::SeqCst)
                .then(|| call(self.name)))
        })
    }
    fn release_for_finalization(&self) -> RunFuture<'_, bool> {
        Box::pin(async {
            self.released.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        (call.name == self.name)
            .then_some(())
            .ok_or(RunError::ToolRejected)
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async {
            Ok(ToolOutput {
                content: "done".into(),
                is_error: false,
            })
        })
    }
    fn result_committed(&self, _: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async {
            self.committed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test]
async fn composed_registry_cancellation_chain_reaches_finalization_owner_and_all_releases() {
    let owner = |name, offer| {
        Arc::new(FinalizationOwner {
            name,
            offer: AtomicBool::new(offer),
            released: AtomicUsize::new(0),
            committed: AtomicUsize::new(0),
            canceled: AtomicUsize::new(0),
        })
    };
    let primary = owner("read", false);
    let fallback = owner("deliver", true);
    let definition = |name: &str| ToolDefinition {
        namespace: None,
        name: name.into(),
        description: name.into(),
        parameters: serde_json::json!({"type":"object"}),
    };
    let definitions = vec![definition("read"), definition("deliver")];
    let composed = ComposedToolExecutor::new(
        primary.clone(),
        [definition("read")],
        Some(fallback.clone()),
        [definition("deliver")],
    );
    let registry =
        ToolRegistry::from_registrations([ToolRegistration::new(definitions, &composed)]).unwrap();
    let cancellation = AgentCancellation::default();
    let wrapper = CancellableTools {
        inner: &registry,
        cancellation: cancellation.clone(),
    };
    assert!(wrapper.supports_parallel(&call("read")));
    assert!(wrapper.cancellation_safe_to_drop(&call("read")));
    assert!(!wrapper.supports_parallel(&call("deliver")));
    assert!(!wrapper.cancellation_safe_to_drop(&call("unknown")));
    let finalization = wrapper.finalization_call().await.unwrap().unwrap();
    assert_eq!(finalization.name, "deliver");
    wrapper
        .authorize_and_execute(finalization.clone())
        .await
        .unwrap();
    cancellation.cancel();
    wrapper.cancel_pending(call("deliver")).await.unwrap();
    assert_eq!(fallback.canceled.load(Ordering::SeqCst), 1);
    assert_eq!(primary.canceled.load(Ordering::SeqCst), 0);
    // Resource handoff and committed-result bookkeeping survive cancellation.
    assert!(wrapper.release_for_finalization().await.unwrap());
    wrapper.result_committed(finalization).await.unwrap();
    assert_eq!(primary.released.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.released.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.committed.load(Ordering::SeqCst), 1);
}
