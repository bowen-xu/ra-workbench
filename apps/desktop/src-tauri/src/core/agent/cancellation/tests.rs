use super::*;
use research_agent_core::{
    model::StreamLimits,
    protocol::{EventPayload, ModelIdentity, TurnOutcome},
    runner::{RunIdSource, RunLimits, TurnEventSink},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[test]
fn cancellation_registry_shares_exclusive_host_policy() {
    let registry = CancellationRegistry::default();
    let first = registry
        .register("run-1", true)
        .expect("first registration");
    assert!(registry.register("run-2", true).is_err());
    assert!(registry.cancel("run-1").expect("cancel"));
    assert!(first.is_cancelled());
    registry.remove("run-1").expect("remove");
    let second = registry
        .register("run-2", true)
        .expect("second registration");
    assert!(!second.is_cancelled());
}

struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct Transport {
    entered: Arc<Notify>,
    drops: Arc<AtomicUsize>,
    wait_at_start: bool,
}
impl ModelTransport for Transport {
    fn tool_output_policy(
        &self,
    ) -> Option<research_agent_core::model::tool_output::ToolOutputPolicy> {
        Some(research_agent_core::model::tool_output::ToolOutputPolicy::Tokens(137))
    }
    fn prepare_context_policy(&self) -> ModelFuture<'_, ()> {
        Box::pin(async move {
            let _guard = DropCount(self.drops.clone());
            self.entered.notify_one();
            std::future::pending().await
        })
    }
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: false,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, _: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            if self.wait_at_start {
                let _guard = DropCount(self.drops.clone());
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(Box::new(Stream {
                entered: self.entered.clone(),
                _guard: DropCount(self.drops.clone()),
            }) as Box<dyn ModelStream>)
        })
    }
}

#[tokio::test]
async fn policy_preparation_is_forwarded_and_cancelable_without_inference() {
    let entered = Arc::new(Notify::new());
    let drops = Arc::new(AtomicUsize::new(0));
    let inner = Transport {
        entered: entered.clone(),
        drops: drops.clone(),
        wait_at_start: true,
    };
    let cancellation = AgentCancellation::default();
    let transport = CancellableTransport {
        inner: &inner,
        cancellation: cancellation.clone(),
    };
    assert_eq!(transport.tool_output_policy(), inner.tool_output_policy());
    let prepare = transport.prepare_context_policy();
    let cancel = async {
        entered.notified().await;
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(prepare, cancel);
    assert_eq!(result, Err(ModelError::Canceled));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
struct Stream {
    entered: Arc<Notify>,
    _guard: DropCount,
}
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move {
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
struct Tools;
struct CompletedTransport;
struct CompletedStream(std::collections::VecDeque<ModelStreamEvent>);
impl ModelStream for CompletedStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async { Ok(self.0.pop_front()) })
    }
}
impl ModelTransport for CompletedTransport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: false,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            Ok(Box::new(CompletedStream(
                vec![
                    ModelStreamEvent::Started {
                        response_id: "completed".into(),
                        actual_model: request.model,
                    },
                    ModelStreamEvent::TextDelta("answer".into()),
                    ModelStreamEvent::Completed {
                        reason: research_agent_core::model::FinishReason::Stop,
                        end_turn: None,
                    },
                ]
                .into(),
            )) as Box<dyn ModelStream>)
        })
    }
}
struct FinalizationProbe {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    finished: AtomicUsize,
    return_call: bool,
    fail: bool,
}
impl ToolExecutor for FinalizationProbe {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        panic!("canceled finalization must not dispatch")
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        panic!("canceled finalization must not dispatch")
    }
    fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            self.finished.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(RunError::ToolOutcomeUnknown);
            }
            Ok(self.return_call.then(|| ToolCall {
                namespace: None,
                raw_arguments_json: None,
                call_id: "finalize".into(),
                name: "finalize".into(),
                arguments: serde_json::json!({}),
            }))
        })
    }
}

#[tokio::test]
async fn cancel_during_finalization_inspection_waits_for_owner_then_records_canceled() {
    for return_call in [false, true] {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let tools = FinalizationProbe {
            entered: entered.clone(),
            release: release.clone(),
            finished: AtomicUsize::new(0),
            return_call,
            fail: false,
        };
        let sink = Sink::default();
        let cancellation = AgentCancellation::default();
        let mut run = Box::pin(run_cancellable_turn(
            TurnRunner {
                transport: &CompletedTransport,
                tools: &tools,
                sink: &sink,
                ids: &Ids,
                limits: limits(),
            },
            Uuid::new_v4(),
            request(),
            cancellation.clone(),
        ));
        tokio::select! { _ = entered.notified() => {}, _ = &mut run => panic!("inspection must be awaited") }
        cancellation.cancel();
        tokio::select! { _ = tokio::task::yield_now() => {}, _ = &mut run => panic!("owner inspection must finish before cancellation") }
        assert_eq!(tools.finished.load(Ordering::SeqCst), 0);
        release.notify_one();
        assert!(matches!(
            run.await,
            Err(RunError::Model(ModelError::Canceled))
        ));
        assert_eq!(tools.finished.load(Ordering::SeqCst), 1);
        assert_canceled(&sink);
    }
}

#[tokio::test]
async fn finalization_inspection_failure_survives_concurrent_cancellation() {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let inner = FinalizationProbe {
        entered: entered.clone(),
        release: release.clone(),
        finished: AtomicUsize::new(0),
        return_call: false,
        fail: true,
    };
    let cancellation = AgentCancellation::default();
    let tools = CancellableTools {
        inner: &inner,
        cancellation: cancellation.clone(),
    };
    let mut inspection = Box::pin(tools.finalization_call());
    tokio::select! { _ = entered.notified() => {}, _ = &mut inspection => panic!("inspection must be awaited") }
    cancellation.cancel();
    release.notify_one();
    assert!(matches!(
        inspection.await,
        Err(RunError::ToolOutcomeUnknown)
    ));
    assert_eq!(inner.finished.load(Ordering::SeqCst), 1);
}
struct ReceiptTools(AtomicUsize);
impl ToolExecutor for ReceiptTools {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        panic!("receipt must not revalidate execution")
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        panic!("receipt must not execute again")
    }
    fn result_committed(&self, _: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async {
            tokio::task::yield_now().await;
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test]
async fn cancelled_turn_still_finishes_durable_result_bookkeeping() {
    let inner = ReceiptTools(AtomicUsize::new(0));
    let cancellation = AgentCancellation::default();
    cancellation.cancel();
    CancellableTools {
        inner: &inner,
        cancellation,
    }
    .result_committed(ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "completed-call".into(),
        name: "exec".into(),
        arguments: serde_json::json!({}),
    })
    .await
    .unwrap();
    assert_eq!(inner.0.load(Ordering::SeqCst), 1);
}

impl ToolExecutor for Tools {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        panic!("not requested")
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        panic!("not requested")
    }
}
#[derive(Default)]
struct Sink {
    events: Mutex<Vec<EventPayload>>,
    gate: Option<(Arc<Notify>, Arc<Notify>)>,
}
impl TurnEventSink for Sink {
    fn emit(&self, event: EventPayload) -> RunFuture<'_, ()> {
        Box::pin(async move {
            if matches!(event, EventPayload::ModelRequestStarted { .. })
                && let Some((entered, release)) = &self.gate
            {
                entered.notify_one();
                release.notified().await;
            }
            self.events.lock().unwrap().push(event);
            Ok(())
        })
    }
}
struct Ids;
impl RunIdSource for Ids {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}
fn request() -> ModelRequest {
    ModelRequest {
        captured_history: None,
        request_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "test".into(),
            revision: None,
        },
        messages: vec![research_agent_core::model::ModelMessage::User {
            content: vec![],
            source_item_id: None,
        }],
        tools: vec![],
        max_output_tokens: 20,
        parallel_tool_calls: false,
        reasoning_effort: None,
    }
}
fn limits() -> RunLimits {
    RunLimits {
        model_calls: None,
        tool_calls: None,
        output_bytes: None,
        tool_output_bytes: 100,
        response: StreamLimits {
            text_bytes: 100,
            argument_bytes: 100,
            tool_calls: 1,
        },
    }
}
fn assert_canceled(sink: &Sink) {
    let events = sink.events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                EventPayload::TurnFinished {
                    outcome: TurnOutcome::Canceled,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(matches!(
        events.last(),
        Some(EventPayload::TurnFinished {
            outcome: TurnOutcome::Canceled,
            ..
        })
    ));
}

#[tokio::test]
async fn cancel_model_start_and_stream_wait_drops_owned_work_and_records_one_terminal() {
    for wait_at_start in [true, false] {
        let entered = Arc::new(Notify::new());
        let drops = Arc::new(AtomicUsize::new(0));
        let transport = Transport {
            entered: entered.clone(),
            drops: drops.clone(),
            wait_at_start,
        };
        let sink = Sink::default();
        let cancellation = AgentCancellation::default();
        let runner = TurnRunner {
            transport: &transport,
            tools: &Tools,
            sink: &sink,
            ids: &Ids,
            limits: limits(),
        };
        let mut run = Box::pin(run_cancellable_turn(
            runner,
            Uuid::new_v4(),
            request(),
            cancellation.clone(),
        ));
        tokio::select! { _=entered.notified()=>{}, _=&mut run=>panic!("must wait for model") }
        cancellation.cancel();
        cancellation.cancel();
        assert!(matches!(
            run.await,
            Err(RunError::Model(ModelError::Canceled))
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_canceled(&sink);
    }
}

#[tokio::test]
async fn cancellation_waits_for_event_commit_instead_of_dropping_it() {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let drops = Arc::new(AtomicUsize::new(0));
    let transport = Transport {
        entered: Arc::new(Notify::new()),
        drops: drops.clone(),
        wait_at_start: true,
    };
    let sink = Sink {
        events: Mutex::new(vec![]),
        gate: Some((entered.clone(), release.clone())),
    };
    let cancellation = AgentCancellation::default();
    let runner = TurnRunner {
        transport: &transport,
        tools: &Tools,
        sink: &sink,
        ids: &Ids,
        limits: limits(),
    };
    let mut run = Box::pin(run_cancellable_turn(
        runner,
        Uuid::new_v4(),
        request(),
        cancellation.clone(),
    ));
    tokio::select! { _=entered.notified()=>{}, _=&mut run=>panic!("must wait for commit") }
    cancellation.cancel();
    // Poll once after cancellation; the commit is still pending, not discarded.
    let pending =
        std::future::poll_fn(|cx| std::task::Poll::Ready(run.as_mut().poll(cx).is_pending())).await;
    assert!(pending);
    release.notify_one();
    assert!(matches!(
        run.await,
        Err(RunError::Model(ModelError::Canceled))
    ));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "model operation must never start"
    );
    assert!(
        sink.events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, EventPayload::ModelRequestStarted { .. }))
    );
    assert_canceled(&sink);
}

#[tokio::test]
async fn cancellation_drops_explicitly_drop_safe_tool_and_prevents_later_execution() {
    struct PendingTools {
        entered: Arc<Notify>,
        drops: Arc<AtomicUsize>,
    }
    impl ToolExecutor for PendingTools {
        fn cancellation_safe_to_drop(&self, _: &ToolCall) -> bool {
            true
        }
        fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
            Ok(())
        }
        fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
            Box::pin(async move {
                let _guard = DropCount(self.drops.clone());
                self.entered.notify_one();
                std::future::pending().await
            })
        }
    }
    let entered = Arc::new(Notify::new());
    let drops = Arc::new(AtomicUsize::new(0));
    let tools = PendingTools {
        entered: entered.clone(),
        drops: drops.clone(),
    };
    let cancellation = AgentCancellation::default();
    let wrapped = CancellableTools {
        inner: &tools,
        cancellation: cancellation.clone(),
    };
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "call".into(),
        name: "read".into(),
        arguments: serde_json::json!({}),
    };
    let mut pending = wrapped.authorize_and_execute(call.clone());
    tokio::select! {_=entered.notified()=>{},_=&mut pending=>panic!("tool must wait")}
    cancellation.cancel();
    assert!(matches!(
        pending.await,
        Err(RunError::Model(ModelError::Canceled))
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(matches!(
        wrapped.validate(&call),
        Err(RunError::Model(ModelError::Canceled))
    ));
}
