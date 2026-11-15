//! A byte-budget rejection must not replace the installed provider window.
use super::*;
use research_agent_core::context::retained_text::{RetainedTextPolicy, Utf8RetainedTextPolicy};

struct NoCandidateIo {
    inner: Arc<dyn CompactionStrategy>,
    calls: Arc<AtomicUsize>,
}
impl CompactionStrategy for NoCandidateIo {
    fn binding(&self) -> CompactionBinding {
        self.inner.binding()
    }
    fn retention_policy(&self) -> Option<CompactionRetention> {
        self.inner.retention_policy()
    }
    fn retained_text_policy(&self) -> Option<Arc<dyn RetainedTextPolicy>> {
        self.inner.retained_text_policy()
    }
    fn validate_output(&self, output: &CompactionOutput) -> Result<(), CompactionFailure> {
        self.inner.validate_output(output)
    }
    fn compact(&self, _: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { panic!("candidate I/O after retained byte rejection") })
    }
}

#[tokio::test]
async fn unfittable_newest_retained_text_fails_without_io_or_window_replacement() {
    let fixture = Fixture::new().await;
    let (url, peer) = sequential_server(vec![ok(&body()), ok(&events(&[opaque()], true))]).await;
    let original = "HEADmiddleTAIL";
    let first = fixture.record(original).await;
    let execution = fixture.execution(&first, fixture.transport(&first, &url));
    assert_eq!(
        fixture
            .start(&first, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let compact = fixture.record("/compact").await;
    let mut execution = fixture.execution(&compact, fixture.transport(&compact, &url));
    let binding = execution.compaction_strategy.as_ref().unwrap().binding();
    execution.compaction_strategy = Some(Arc::new(
        ResponsesV2CompactionStrategy::new(fixture.transport(&compact, &url), binding)
            .unwrap()
            .with_retained_text_policy(Arc::new(Utf8RetainedTextPolicy { max_tokens: 2 }))
            .unwrap(),
    ));
    assert_eq!(
        fixture
            .start(&compact, execution, true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let before = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(peer.await.unwrap().len(), 2);

    let rejected = fixture.record("/compact").await;
    let mut execution = fixture.execution(&rejected, fixture.transport(&rejected, &url));
    // The authority fits, but its remaining retention allocation cannot hold
    // even the newest supplier truncation marker. No transport request follows.
    execution.max_input_bytes = 100;
    let calls = Arc::new(AtomicUsize::new(0));
    execution.compaction_strategy = Some(Arc::new(NoCandidateIo {
        inner: execution.compaction_strategy.take().unwrap(),
        calls: calls.clone(),
    }));
    assert_eq!(
        fixture
            .start(&rejected, execution, true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Failed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let after = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.window_id, before.window_id);
    assert!(after.output == before.output && after.retained == before.retained);
    let history = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    assert!(
        history
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| item.text == original)
    );
    let (next_url, next_peer) = sequential_server(vec![ok(&body())]).await;
    let next = fixture.record("continue after failed compact").await;
    let execution = fixture.execution(&next, fixture.transport(&next, &next_url));
    assert_eq!(
        fixture
            .start(&next, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let requests = next_peer.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| **item == opaque())
            .count(),
        1
    );
    assert!(
        requests[0]
            .to_string()
            .contains("HEAD…2 tokens truncated…TAIL")
    );
    assert!(!requests[0].to_string().contains(original));
}
