use super::*;
use research_agent_core::context::{checkpoint::ContextCheckpoint, compaction_strategy::*};
use research_agent_core::model::ContextPersistenceDiagnostic;

fn persistence(id: Uuid) -> ModelError {
    ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id: id,
        phase: "commit",
        safe_cause: "sql_commit_outcome_unknown",
        sqlite_extended_code: Some(787),
    })
}
struct ValidationFailure {
    inner: Arc<Transport>,
    conversation: Uuid,
}
impl ModelTransport for ValidationFailure {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
    fn validate_compaction_window<'a>(
        &'a self,
        _: &'a ContextCheckpoint,
        _: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async move { Err(persistence(self.conversation)) })
    }
}
struct SupplierFailure {
    conversation: Uuid,
}
impl CompactionStrategy for SupplierFailure {
    fn binding(&self) -> CompactionBinding {
        CompactionBinding {
            model: model(),
            strategy_id: "fixture.persistence".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        }
    }
    fn validate_output(&self, _: &CompactionOutput) -> Result<(), CompactionFailure> {
        Ok(())
    }
    fn compact(&self, _: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move { Err(CompactionFailure::Model(persistence(self.conversation))) })
    }
}

#[tokio::test]
async fn compaction_storage_failures_do_not_write_model_failure_or_lose_diagnostic() {
    for supplier in [false, true] {
        let (service, store, transport, _, execution, id) = setup().await;
        let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
        let execution = Arc::new(execution);
        assert_eq!(
            run(
                execution.clone(),
                service.clone(),
                store.clone(),
                id,
                first.request_id
            )
            .await,
            TaskCompletion::Completed
        );
        let mut execution = Arc::try_unwrap(execution).ok().unwrap();
        if supplier {
            execution.compaction_strategy = Some(Arc::new(SupplierFailure { conversation: id }));
        } else {
            execution.transport = Arc::new(ValidationFailure {
                inner: transport.clone(),
                conversation: id,
            });
        }
        let (record, admission) =
            compact(Arc::new(execution), service.clone(), store.clone(), id).await;
        let SubmissionAdmission::Started(owned) = admission else {
            panic!("new task expected")
        };
        let observed = owned.task.observe().await;
        assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
        let detail = serde_json::to_value(observed.diagnostic.unwrap()).unwrap();
        assert_eq!(detail["conversationId"], id.to_string());
        assert_eq!(detail["safeCause"], "sql_commit_outcome_unknown");
        let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
        let turn = history.turn(record.turn_id).unwrap();
        assert_eq!(turn.outcome, Some(TurnOutcome::Interrupted));
        assert_eq!(turn.attempts.len(), 1);
        assert!(
            turn.attempts
                .values()
                .all(|attempt| attempt.failure_category.is_none())
        );
        assert!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .is_none()
        );
    }
}
