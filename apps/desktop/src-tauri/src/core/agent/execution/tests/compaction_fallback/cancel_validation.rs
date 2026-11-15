use super::*;
use research_agent_core::context::checkpoint::ContextCheckpoint;

struct CancelValidation(Arc<Transport>);
impl ModelTransport for CancelValidation {
    fn capabilities(&self) -> ModelCapabilities {
        self.0.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.0.start(request)
    }
    fn validate_compaction_window<'a>(
        &'a self,
        _: &'a ContextCheckpoint,
        _: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async { Err(ModelError::Canceled) })
    }
}

#[tokio::test]
async fn canceled_validation_wins_over_previous_fallback_error() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    let old = Arc::new(Strategy {
        producer: ModelIdentity {
            model_id: "old-producer".into(),
            ..model()
        },
        calls: AtomicUsize::new(0),
        fails: true,
        fallback: true,
        pending: None,
    });
    let current = Arc::new(Strategy {
        producer: model(),
        calls: AtomicUsize::new(0),
        fails: false,
        fallback: false,
        pending: None,
    });
    execution.compaction_strategy = Some(current.clone());
    execution.transport = Arc::new(CancelValidation(transport.clone()));
    let record = store
        .put(id, Uuid::new_v4(), input("fallback"))
        .await
        .unwrap();
    let worker = service.clone();
    let selected = old.clone();
    let request = record.request_id;
    let admission = service
        .start_submission(
            store.as_ref(),
            ConversationId::from_uuid(id),
            request,
            move |record, _, signal| async move {
                let history = worker.open(ConversationId::from_uuid(id)).await.unwrap();
                let strategy: Arc<dyn CompactionStrategy> = selected;
                let source = [ModelMessage::User {
                    content: vec![ContentPart::Text("prior evidence".into())],
                    source_item_id: None,
                }];
                let result = execution
                    .compact_history_selected(
                        &worker,
                        &record,
                        &history,
                        None,
                        CompactionPhase::PreTurn,
                        &signal,
                        &source,
                        &[],
                        &[],
                        &HashMap::new(),
                        Some(&strategy),
                        false,
                    )
                    .await;
                assert!(matches!(result, Err(RunError::Model(ModelError::Canceled))));
                ServiceTurnSink {
                    service: &worker,
                    conversation_id: ConversationId::from_uuid(id),
                    request_id: request,
                }
                .emit(EventPayload::TurnFinished {
                    turn_id: record.turn_id,
                    outcome: TurnOutcome::Canceled,
                })
                .await?;
                result.map(|_| ())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new task expected")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Canceled);
    assert_eq!(old.calls.load(Ordering::SeqCst), 1);
    assert_eq!(current.calls.load(Ordering::SeqCst), 1);
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Canceled)
    );
}
