use super::*;
struct Denied(Arc<ResponsesTransport>);
impl ModelTransport for Denied {
    fn capabilities(&self) -> ModelCapabilities {
        self.0.capabilities()
    }
    fn supports_compaction_kind(&self, kind: CompactionKind) -> bool {
        self.0.supports_compaction_kind(kind)
    }
    fn authorize_request<'a>(&'a self, _: &'a ModelRequest) -> ModelFuture<'a, ()> {
        Box::pin(async { Err(ModelError::Authentication) })
    }
    fn start(&self, _: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async { panic!("disclosure after authority rejection") })
    }
}
struct CandidateCalls {
    inner: Arc<dyn CompactionStrategy>,
    count: Arc<AtomicUsize>,
}
impl CompactionStrategy for CandidateCalls {
    fn binding(&self) -> CompactionBinding {
        self.inner.binding()
    }
    fn retention_policy(&self) -> Option<CompactionRetention> {
        self.inner.retention_policy()
    }
    fn validate_output(&self, output: &CompactionOutput) -> Result<(), CompactionFailure> {
        self.inner.validate_output(output)
    }
    fn compact(&self, _: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { panic!("candidate I/O after authority rejection") })
    }
}
#[tokio::test]
async fn native_remote_compaction_revalidates_host_authority_before_candidate_io() {
    let fixture = Fixture::new().await;
    let record = fixture.record("/compact").await;
    let inner = fixture.transport(&record, "http://127.0.0.1:1/responses");
    let mut execution = fixture.execution(&record, inner.clone());
    let count = Arc::new(AtomicUsize::new(0));
    execution.compaction_strategy = Some(Arc::new(CandidateCalls {
        inner: execution.compaction_strategy.take().unwrap(),
        count: count.clone(),
    }));
    execution.transport = Arc::new(Denied(inner));
    assert_eq!(
        fixture
            .start(&record, execution, true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Failed
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(
        fixture
            .service
            .active_context_checkpoint(fixture.id)
            .await
            .unwrap()
            .is_none()
    );
}
