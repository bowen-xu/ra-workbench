use super::*;
use research_agent_core::context::compaction_hooks::{
    CompactionHookNotification, CompactionHookOutcome, CompactionHookRequest, CompactionHookResult,
    CompactionHookStage,
};
#[path = "compaction_hooks/handlers.rs"]
mod handlers;

impl AgentExecution {
    pub(super) async fn run_compact_session_start(
        &self,
        service: &AgentSessionService,
        record: &SubmissionRecord,
        signal: &AgentCancellation,
    ) -> Result<Vec<ModelMessage>, RunError> {
        if signal.is_cancelled() {
            return Err(ModelError::Canceled.into());
        }
        // Await the consume transaction rather than selecting/dropping it. The
        // existing conversation task owns dispatch; no second hook runner exists.
        while let Some(window) = service
            .take_compact_session_start(record.conversation_id)
            .await
            .map_err(|error| match error {
                research_agent_core::context::checkpoint_store::CheckpointStoreError::PersistenceDiagnostic(detail) => RunError::from(ModelError::PersistenceDiagnostic(detail)),
                _ => RunError::EventSink,
            })?
        {
            self.run_compaction_hook(
                service,
                CompactionHookRequest {
                    operation_id: window.window_id,
                    conversation_id: record.conversation_id,
                    turn_id: record.turn_id,
                    phase: window.phase,
                    model: self.model.clone(),
                    stage: CompactionHookStage::SessionStartCompact,
                },
                signal,
            )
            .await?;
        }
        let contexts = service
            .compact_hook_contexts(record.conversation_id)
            .await
            .map_err(|_| RunError::EventSink)?;
        Ok(contexts
            .into_iter()
            .filter(|batch| batch.turn_id == record.turn_id)
            .flat_map(|batch| batch.contexts.into_iter().map(ModelMessage::Developer))
            .collect())
    }
    pub(super) async fn run_compaction_hook(
        &self,
        service: &AgentSessionService,
        request: CompactionHookRequest,
        signal: &AgentCancellation,
    ) -> Result<(), RunError> {
        if signal.is_cancelled() {
            return Err(ModelError::Canceled.into());
        }
        let Some(hooks) = self.compaction_hooks.as_ref() else {
            return Ok(());
        };
        hooks
            .validate_request(&request)
            .map_err(|_| RunError::Model(ModelError::Unavailable))?;
        let handler_ids = hooks.handler_ids_for(&request);
        let collector = Arc::new(
            handlers::HandlerCollector::new(request.clone(), handler_ids.clone())
                .ok_or(RunError::EventSink)?,
        );
        service
            .record_compaction_hook(
                request.clone(),
                CompactionHookNotification::Started,
                handler_ids,
            )
            .await
            .map_err(|_| RunError::EventSink)?;
        hooks.notify(&request, CompactionHookNotification::Started);
        let result = tokio::select! {
            biased;
            _ = signal.cancelled() => CompactionHookResult::without_context(CompactionHookOutcome::Canceled),
            result = hooks.run_with_handler_observer(request.clone(), collector.clone()) => result,
        };
        let outcome = result.outcome;
        let handlers = collector.completed(outcome).ok_or(RunError::EventSink)?;
        let result = hooks
            .prepare_context(request.clone(), result)
            .await
            .map_err(|_| RunError::EventSink)?;
        if result.outcome != outcome {
            return Err(RunError::EventSink);
        }
        service
            .record_compaction_hook_result(request.clone(), result, handlers)
            .await
            .map_err(|_| RunError::EventSink)?;
        hooks.notify(&request, CompactionHookNotification::Completed(outcome));
        if signal.is_cancelled()
            || matches!(
                outcome,
                CompactionHookOutcome::Stop | CompactionHookOutcome::Canceled
            )
        {
            // The application represents upstream TurnAborted as a canceled
            // task. A PostCompact stop does not undo the installed checkpoint.
            Err(ModelError::Canceled.into())
        } else {
            Ok(())
        }
    }
}
