use super::*;
use research_agent_core::context::compaction_strategy::*;

struct CapturedCatalog {
    inner: Arc<dyn CompactionStrategy>,
    tools: Mutex<Vec<ToolDefinition>>,
}
impl CompactionStrategy for CapturedCatalog {
    fn binding(&self) -> CompactionBinding {
        self.inner.binding()
    }
    fn text_compaction_prompt(&self) -> Option<&str> {
        self.inner.text_compaction_prompt()
    }
    fn validate_output(&self, output: &CompactionOutput) -> Result<(), CompactionFailure> {
        self.inner.validate_output(output)
    }
    fn compact(&self, request: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move {
            *self.tools.lock().unwrap() = request.source.tools.clone();
            self.inner.compact(request).await
        })
    }
}

#[tokio::test]
async fn manual_task_passes_frozen_catalog_to_strategy_while_text_transport_omits_tools() {
    let (service, store, transport, tools, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let captured = Arc::new(CapturedCatalog {
        inner: execution.compaction_strategy.take().unwrap(),
        tools: Mutex::new(vec![]),
    });
    execution.compaction_strategy = Some(captured.clone());
    let (_, admission) = compact(Arc::new(execution), service.clone(), store, id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let catalog = captured.tools.lock().unwrap();
    assert_eq!(catalog.len(), 4);
    assert_eq!(catalog[0].name, "read");
    assert!(
        catalog
            .iter()
            .any(|definition| definition.name == "research_read_source")
    );
    // Provider policy controls advertising, not whether the task receives the
    // catalog. The textual strategy keeps its Codex-compatible empty tool list.
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].tools.is_empty());
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
}
