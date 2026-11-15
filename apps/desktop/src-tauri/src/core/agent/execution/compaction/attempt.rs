//! Task-owned attempts: supplier policy authorizes a single model fallback.
use super::*;
use research_agent_core::context::compaction_strategy::{CompactionCandidate, CompactionStrategy};
use research_agent_core::context::remote_compaction_input::prepare_model_input;

pub(super) struct CompletedAttempt {
    pub candidate: CompactionCandidate,
    pub attempt_id: Uuid,
    pub producer: ModelIdentity,
    pub expected_revision: u64,
    pub original_error: Option<RunError>,
}

impl AgentExecution {
    pub(super) async fn compaction_attempt(
        &self,
        selected: &Arc<dyn CompactionStrategy>,
        mut request: CompactionRequest,
        previous: Option<&ContextCheckpoint>,
        history: &ConversationProjection,
        sink: &ServiceTurnSink<'_>,
        signal: &AgentCancellation,
    ) -> Result<CompletedAttempt, RunError> {
        let mut strategy = selected;
        let mut original_error = None;
        loop {
            let attempt_id = request.operation_id;
            let expected_revision = sink
                .service
                .open(sink.conversation_id)
                .await
                .map_err(|_| RunError::EventSink)?
                .seq();
            let operation = async {
                self.transport.authorize_request(&request.source).await?;
                let mut prepared = request.clone();
                if let Some(policy) = strategy.tool_output_policy() {
                    super::super::request::project_history_tool_results(
                        &mut prepared.source.messages,
                        history,
                        Some(policy),
                        self.limits.tool_output_bytes,
                    )
                    .map_err(|error| match error {
                        RunError::Budget => CompactionFailure::InvalidBudget,
                        RunError::Model(error) => CompactionFailure::Model(error),
                        _ => CompactionFailure::InvalidOutput,
                    })?;
                }
                if prepared.binding.kind != CompactionKind::TextSummary {
                    let producer = strategy.execution_model();
                    let accounting = strategy.input_accounting().or_else(|| {
                        (producer == self.model)
                            .then(|| self.transport.context_accounting())
                            .flatten()
                    });
                    if let Some(accounting) = accounting {
                        prepared.source.messages = prepare_model_input(
                            &producer,
                            &accounting,
                            &prepared.source.messages,
                            &prepared.source.tools,
                            previous,
                        )?;
                    }
                }
                prepare_compaction(strategy.as_ref(), prepared).await
            };
            let result = tokio::select! {
                biased;
                _ = signal.cancelled() => Err(ModelError::Canceled.into()),
                result = operation => result.map_err(compaction_error),
            };
            match result {
                Ok(candidate) => {
                    return Ok(CompletedAttempt {
                        candidate,
                        attempt_id,
                        producer: strategy.execution_model(),
                        expected_revision,
                        original_error,
                    });
                }
                Err(error) if error.is_event_sink() => return Err(error),
                Err(error) => {
                    sink.emit(EventPayload::ModelRequestFailed {
                        turn_id: request.turn_id,
                        attempt_id,
                        error: error.to_string(),
                        retryable: false,
                        category: Some(error.failure_category()),
                    })
                    .await?;
                    if signal.is_cancelled()
                        || matches!(error, RunError::Model(ModelError::Canceled))
                    {
                        return Err(ModelError::Canceled.into());
                    }
                    let allowed = original_error.is_none()
                        && matches!(&error, RunError::Model(error) if strategy.allows_current_model_fallback(error));
                    let current = self.compaction_strategy.as_ref().filter(|current| {
                        current.binding() == request.binding
                            && current.execution_model() == self.model
                            && current.retention_policy() == strategy.retention_policy()
                    });
                    if !allowed || current.is_none() {
                        return Err(original_error.unwrap_or(error));
                    }
                    // Fresh ownership identifiers keep the failed old attempt
                    // separate from the current-model request and checkpoint.
                    original_error = Some(error);
                    strategy = current.ok_or(ModelError::UnsupportedCapability)?;
                    request.operation_id = Uuid::new_v4();
                    request.source.request_id = request.operation_id;
                    sink.emit(EventPayload::ModelRequestStarted {
                        turn_id: request.turn_id,
                        attempt_id: request.operation_id,
                        model: strategy.execution_model(),
                    })
                    .await?;
                }
            }
        }
    }
}
