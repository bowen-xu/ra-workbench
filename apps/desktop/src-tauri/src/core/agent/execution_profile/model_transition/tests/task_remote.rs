//! Real task, accounting, remote codecs and durable windows; synthetic network.
use super::*;
#[path = "task_remote/cancel.rs"]
mod cancel;
#[path = "task_remote/failure.rs"]
mod failure;
#[path = "task_remote/harness.rs"]
mod harness;
use harness::*;
use research_agent_core::{context::checkpoint::ContextCheckpoint, protocol::TurnOutcome};
use serde_json::{Value, json};

// Observe which estimator reaches the actual task-owned previous-model attempt.
struct ProducerTransport {
    inner: Arc<dyn ModelTransport>,
    accounting: Arc<research_agent_core::context::accounting::ContextAccounting>,
}
impl ModelTransport for ProducerTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn authorize_request<'a>(&'a self, request: &'a ModelRequest) -> ModelFuture<'a, ()> {
        self.inner.authorize_request(request)
    }
    fn context_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        Some(self.accounting.clone())
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}
struct ProducerEstimator {
    messages: Arc<std::sync::atomic::AtomicUsize>,
    wire: Arc<std::sync::atomic::AtomicUsize>,
}
impl research_agent_core::context::accounting::ContextTokenEstimator for ProducerEstimator {
    fn estimate(&self, _: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
        self.messages
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(1)
    }
    fn estimate_compaction(&self, output: &CompactionOutput) -> Option<u64> {
        if matches!(output, CompactionOutput::RemoteWindow(_)) {
            self.wire.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Some(1)
    }
}

#[tokio::test]
async fn native_remote_fallback_installs_and_samples_the_current_window() {
    for old_native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        for native in [
            NativeCompactionPolicy::ChatGptResponsesV2,
            NativeCompactionPolicy::ChatGptResponsesFullWindow,
        ] {
            exercise(old_native, native).await;
        }
    }
}

async fn exercise(old_native: NativeCompactionPolicy, native: NativeCompactionPolicy) {
    let mut fixture = Fixture::new().await;
    let old_model = model("old");
    let current_model = model("current");
    let old_item = json!({"type":"compaction","encrypted_content":"old-installed-state"});
    let new_item = json!({"type":"compaction","encrypted_content":"current-installed-state","vendor_extra":{"version":2}});
    let failed = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
        wire(
            "text/event-stream",
            [json!({"type":"response.created","response":{"id":"old-failed","model":"gpt-5.6-terra"}}),
             json!({"type":"error","code":"context_length_exceeded","message":"synthetic"})]
                .into_iter().map(|event| format!("data: {event}\n\n")).collect(),
        )
    } else {
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
    };
    let (old_url, old_peer) = server(vec![
        sample("old-response"),
        compact(old_native, &old_item),
        failed,
    ])
    .await;
    let (current_url, current_peer) = server(vec![
        compact_for(native, &new_item, &current_model.model_id),
        sample_for("current-response", &current_model.model_id),
        sample_for("resumed-response", &current_model.model_id),
    ])
    .await;
    let first = fixture
        .record(old_model.clone(), "historical evidence")
        .await;
    let old_transport = fixture.transport(&first, &old_url, old_native);
    let first_execution = fixture.execution(&first, old_transport.clone(), old_native);
    let estimates = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wire_estimates = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let old_accounted_transport = Arc::new(ProducerTransport {
        inner: first_execution.transport.clone(),
        accounting: Arc::new(
            research_agent_core::context::accounting::ContextAccounting {
                model: old_model.clone(),
                budget: Some(CompactionBudget {
                    context_window_tokens: 2000,
                    effective_percent: 95,
                    configured_trigger_tokens: None,
                    output_reserve_tokens: 128,
                }),
                unknown_window_trigger_tokens: None,
                estimator: Arc::new(ProducerEstimator {
                    messages: estimates.clone(),
                    wire: wire_estimates.clone(),
                }),
            },
        ),
    });
    fixture.run(&first, first_execution, false).await;
    let manual = fixture.record(old_model.clone(), "/compact").await;
    let execution = fixture.execution(
        &manual,
        fixture.transport(&manual, &old_url, old_native),
        old_native,
    );
    fixture.run(&manual, execution, true).await;
    let old_window = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old_window.binding.model, old_model);

    let next = fixture
        .record(current_model.clone(), "fresh submission")
        .await;
    let raw = fixture.transport(&next, &current_url, native);
    let current_estimates = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let current_wire = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let current_accounting = Arc::new(
        research_agent_core::context::accounting::ContextAccounting {
            model: current_model.clone(),
            budget: Some(CompactionBudget {
                context_window_tokens: 10000,
                effective_percent: 95,
                configured_trigger_tokens: None,
                output_reserve_tokens: 128,
            }),
            unknown_window_trigger_tokens: None,
            estimator: Arc::new(ProducerEstimator {
                messages: current_estimates.clone(),
                wire: current_wire.clone(),
            }),
        },
    );
    let mut execution =
        fixture.execution_with_accounting(&next, raw, native, current_accounting.clone());
    assert!(Arc::ptr_eq(
        &execution
            .compaction_strategy
            .as_ref()
            .unwrap()
            .input_accounting()
            .unwrap(),
        &current_accounting,
    ));
    let old_strategy = native
        .responses_strategy_with_accounting(
            old_transport.clone(),
            old_model.clone(),
            old_accounted_transport.accounting.clone(),
        )
        .unwrap();
    let selected = Arc::new(
        PreviousModelStrategy::compose(
            old_strategy,
            old_accounted_transport,
            native.binding(current_model.clone()),
            None,
            64,
            true,
        )
        .unwrap(),
    );
    let producer_accounting = selected.input_accounting().unwrap();
    assert_eq!(producer_accounting.model, old_model);
    assert_eq!(
        producer_accounting.budget.unwrap().context_window_tokens,
        2000
    );
    assert_ne!(producer_accounting.model, execution.model);
    execution.transport = Arc::new(Selected {
        inner: execution.transport.clone(),
        selected,
    });
    fixture.run(&next, execution, false).await;
    assert_eq!(estimates.load(std::sync::atomic::Ordering::SeqCst), 2);
    // Full-window checkpoints also need a retained-prefix estimate before the
    // raw HTTP window estimate; V2's opaque prefix uses the separate shape.
    assert_eq!(
        wire_estimates.load(std::sync::atomic::Ordering::SeqCst),
        if old_native == NativeCompactionPolicy::ChatGptResponsesFullWindow {
            2
        } else {
            1
        },
    );
    assert!(current_estimates.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert!(current_wire.load(std::sync::atomic::Ordering::SeqCst) > 0);
    let installed = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(installed.previous_window_id, Some(old_window.window_id));
    assert_eq!(installed.binding.model, current_model);
    assert_eq!(
        installed.credential_scope_id,
        Some(uuid::Uuid::from_u128(1))
    );
    let expected = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
        CompactionOutput::RemoteOpaque(new_item.clone())
    } else {
        CompactionOutput::RemoteWindow(vec![new_item.clone()])
    };
    assert!(installed.output == expected);
    let projection = fixture
        .service
        .open(research_agent_core::conversation::ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    let turn = projection.turn(next.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    let attempts: Vec<_> = turn
        .attempt_order
        .iter()
        .map(|id| (*id, turn.attempts[id].requested_model.clone()))
        .collect();
    assert_eq!(attempts.len(), 3);
    assert_eq!(attempts[0].1, old_model);
    assert_eq!(attempts[1].1, current_model);
    assert_eq!(attempts[2].1, current_model);
    assert!(turn.items[&installed.window_id].completed);
    assert_eq!(
        turn.items[&installed.window_id].kind,
        research_agent_core::protocol::ItemKind::ContextCompaction
    );
    assert!(attempts.iter().all(|(id, _)| *id != installed.window_id));
    assert_ne!(attempts[0].0, attempts[1].0);
    // A new native service and SQLite connection restore the actual checkpoint.
    fixture.reopen().await;
    let saved: ContextCheckpoint = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(&installed).unwrap()
    );
    let resumed = fixture
        .record(current_model, "continue after checkpoint")
        .await;
    let execution = fixture.execution(
        &resumed,
        fixture.transport(&resumed, &current_url, native),
        native,
    );
    fixture.run(&resumed, execution, false).await;

    let old_requests = old_peer.await.unwrap();
    assert_eq!(old_requests.len(), 3);
    assert!(
        old_requests[2]["input"]
            .as_array()
            .unwrap()
            .contains(&old_item)
    );
    assert!(!old_requests[2].to_string().contains("fresh submission"));
    let current_requests = current_peer.await.unwrap();
    assert_eq!(current_requests.len(), 3);
    assert!(
        current_requests[0]["input"]
            .as_array()
            .unwrap()
            .contains(&old_item)
    );
    assert!(!current_requests[0].to_string().contains("fresh submission"));
    for request in &current_requests[1..] {
        let input = request["input"].as_array().unwrap();
        assert!(input.contains(&new_item));
        assert!(!input.contains(&old_item));
        assert_eq!(request.to_string().matches("current authority").count(), 1);
    }
    assert!(current_requests[1].to_string().contains("fresh submission"));
    assert!(
        current_requests[2]
            .to_string()
            .contains("continue after checkpoint")
    );
}
