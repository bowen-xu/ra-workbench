//! Manual CompactTask execution under the existing durable turn owner. The
//! command record supplies authorized model/context identities, never user text.
use super::*;

/// The host confirms a specific journal revision for model disclosure. This
/// scope is checked under the durable session owner before starting inference.
pub struct CompactionStart {
    pub conversation_id: ConversationId,
    pub request_id: Uuid,
    pub expected_revision: u64,
}

impl AgentExecution {
    pub async fn start_compaction(
        self: Arc<Self>,
        service: Arc<AgentSessionService>,
        submissions: Arc<dyn SubmissionStore>,
        id: ConversationId,
        request_id: Uuid,
        materializer: Option<Arc<dyn research_agent_core::submission::ImageMaterializer>>,
    ) -> Result<SubmissionAdmission, SubmissionRunError> {
        let expected_revision = service.open(id).await?.seq();
        self.start_confirmed_compaction(
            service,
            submissions,
            CompactionStart {
                conversation_id: id,
                request_id,
                expected_revision,
            },
            materializer,
        )
        .await
    }

    pub async fn start_confirmed_compaction(
        self: Arc<Self>,
        service: Arc<AgentSessionService>,
        submissions: Arc<dyn SubmissionStore>,
        scope: CompactionStart,
        materializer: Option<Arc<dyn research_agent_core::submission::ImageMaterializer>>,
    ) -> Result<SubmissionAdmission, SubmissionRunError> {
        let CompactionStart {
            conversation_id: id,
            request_id,
            expected_revision,
        } = scope;
        let worker = service.clone();
        let store = submissions.clone();
        service
            .start_compact_submission(
                submissions.as_ref(),
                id,
                request_id,
                expected_revision,
                move |record, history, signal| async move {
                    let sink = ServiceTurnSink {
                        service: &worker,
                        conversation_id: id,
                        request_id,
                    };
                    let mut retained_images = HashMap::new();
                    let result = async {
                        // Advertise the same frozen catalog as an ordinary turn,
                        // without entering the tool execution loop below.
                        let token_budget = token_budget_tools::NativeTokenBudgetTools::new(self.as_ref(), &worker, &record, &signal);
                        let budget_tools = research_agent_core::context::token_budget::tools::TokenBudgetTools::new(token_budget.as_ref().map(|runtime| runtime as &dyn research_agent_core::context::token_budget::tools::TokenBudgetToolRuntime));
                        let source_tools = super::super::source_tools::SubmissionTools { record: &record };
                        let tool_registry = ToolRegistry::from_registrations([
                            self.host_tools.borrowed(),
                            budget_tools.registration(),
                            ToolRegistration::new(if record.input.context.iter().any(|source|super::super::context_session::resource_grants::resource(source).is_some()) {vec![]} else {super::super::source_tools::definitions(&record)}, &source_tools),
                        ]).map_err(|_| RunError::Model(ModelError::InvalidRequest))?;
                        self.request(
                            &worker,
                            store.as_ref(),
                            &record,
                            &history,
                            RequestPreparation {
                                canonical_out: None,
                                retained_images_out: &mut retained_images,
                                compact_only: true,
                                signal: &signal,
                                materializer: materializer.as_deref(),
                                tool_definitions: tool_registry.definitions(),
                            },
                        )
                        .await
                    }.await;
                    // Never run the normal model/tool loop on this replacement. A
                    // subsequent ordinary turn restores current canonical context.
                    let result = result.and_then(|_| {
                        if signal.is_cancelled() {
                            Err(ModelError::Canceled.into())
                        } else {
                            Ok(())
                        }
                    });
                    if result.as_ref().err().is_some_and(RunError::is_event_sink) {
                        return result;
                    }
                    let current = worker.open(id).await.map_err(|_| RunError::EventSink)?;
                    let has_attempt = !current
                        .turn(record.turn_id)
                        .ok_or(RunError::EventSink)?
                        .attempts
                        .is_empty();
                    let no_model_reset = self.compaction_strategy.as_ref().is_some_and(|strategy| {
                        strategy.binding().kind == research_agent_core::context::compaction_strategy::CompactionKind::ResetWindow
                    });
                    if let Err(error) = &result
                        && !has_attempt
                        && !no_model_reset
                        && !matches!(error, RunError::Model(ModelError::Canceled))
                    {
                        // Model-based preparation failures keep their diagnostic attempt.
                        // A no-model reset must never fabricate inference evidence.
                        let attempt_id = Uuid::new_v4();
                        sink.emit(EventPayload::PhaseChanged {
                            turn_id: record.turn_id,
                            phase: research_agent_core::protocol::TurnPhase::RunningModel,
                        })
                        .await?;
                        sink.emit(EventPayload::ModelRequestStarted {
                            turn_id: record.turn_id,
                            attempt_id,
                            model: record.input.model.clone(),
                        })
                        .await?;
                        sink.emit(EventPayload::ModelRequestFailed {
                            turn_id: record.turn_id,
                            attempt_id,
                            error: error.to_string(),
                            retryable: false,
                            category: Some(error.failure_category()),
                        })
                        .await?;
                    }
                    let outcome = match &result {
                        Ok(()) => TurnOutcome::Completed,
                        Err(RunError::Model(ModelError::Canceled)) => TurnOutcome::Canceled,
                        Err(_) => TurnOutcome::Failed,
                    };
                    sink.emit(EventPayload::TurnFinished {
                        turn_id: record.turn_id,
                        outcome,
                    })
                    .await?;
                    result
                },
            )
            .await
    }
}
