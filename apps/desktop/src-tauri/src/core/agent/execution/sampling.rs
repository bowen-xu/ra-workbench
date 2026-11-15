//! Native adapter for Codex's safe post-tool-batch compaction boundary. The
//! shared runner remains the sole sampling, queue and terminal-event owner.
use super::*;
use research_agent_core::context::compaction_strategy::CompactionPhase;

pub(super) struct NativeSamplingContext<'a> {
    pub execution: &'a AgentExecution,
    pub service: &'a AgentSessionService,
    pub submissions: &'a dyn SubmissionStore,
    pub record: &'a SubmissionRecord,
    pub signal: &'a AgentCancellation,
    pub canonical: &'a [ModelMessage],
    pub retained_images: &'a RetainedUserImages,
    pub token_budget: Option<&'a super::token_budget_tools::NativeTokenBudgetTools<'a>>,
    #[cfg(target_os = "macos")]
    pub stop_hooks: Option<&'a dyn StopHooks>,
}
impl SamplingContext for NativeSamplingContext<'_> {
    #[cfg(target_os = "macos")]
    fn stop_hooks(&self) -> Option<&dyn StopHooks> {
        self.stop_hooks
    }
    fn observe_item<'a>(&'a self, turn_id: Uuid, request: &'a ModelRequest) -> RunFuture<'a, ()> {
        Box::pin(async move {
            if turn_id != self.record.turn_id || request.model != self.execution.model {
                return Err(ModelError::InvalidRequest.into());
            }
            if let Some(runtime) = self.token_budget {
                runtime.observe(turn_id, request)?;
            }
            Ok(())
        })
    }
    fn observe_natural_end<'a>(
        &'a self,
        turn: Uuid,
        request: &'a ModelRequest,
    ) -> RunFuture<'a, ()> {
        Box::pin(async move {
            if turn != self.record.turn_id || request.model != self.execution.model {
                return Err(ModelError::InvalidRequest.into());
            }
            if self.signal.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            if self.token_budget.is_none() {
                return Ok(());
            }
            let previous = self
                .service
                .active_context_checkpoint(self.record.conversation_id)
                .await
                .map_err(|_| RunError::EventSink)?;
            let previous = self.execution.consuming_checkpoint(previous)?;
            self.budget_decision(turn, request, previous.as_ref(), false)
                .await?;
            Ok(())
        })
    }
    fn observe_response<'a>(
        &'a self,
        turn_id: Uuid,
        request: &'a ModelRequest,
        response: &'a ModelResponse,
    ) -> RunFuture<'a, ()> {
        Box::pin(async move {
            if turn_id != self.record.turn_id || request.model != self.execution.model {
                return Err(ModelError::InvalidRequest.into());
            }
            let window = self
                .service
                .active_context_checkpoint(self.record.conversation_id)
                .await
                .map_err(|_| RunError::EventSink)?;
            let window = self.execution.consuming_checkpoint(window)?;
            let accounting = self.execution.transport.context_accounting();
            if let Some(accounting) = &accounting {
                accounting.observe_response(request, response, window.as_ref())?;
                accounting
                    .estimator
                    .persist_auto_compaction(window.as_ref())
                    .await?;
            }
            let estimated_tokens = if !response.actual_model_observed
                || response.actual_model != request.model
                || response
                    .usage
                    .input_tokens
                    .zip(response.usage.output_tokens)
                    .is_none()
            {
                accounting
                    .as_ref()
                    .map(|accounting| {
                        accounting.estimate_window(
                            &request.model,
                            &request.messages,
                            &request.tools,
                            window.as_ref(),
                        )
                    })
                    .transpose()?
                    .flatten()
            } else {
                None
            };
            self.service
                .record_context_usage(
                    self.record.conversation_id,
                    crate::agent::context_usage::WindowUsageSample {
                        turn_id,
                        request_id: request.request_id,
                        window_id: window.as_ref().map(|window| window.window_id),
                        model: request.model.clone(),
                        actual_model: response.actual_model.clone(),
                        actual_model_observed: response.actual_model_observed,
                        input_tokens: response.usage.input_tokens,
                        output_tokens: response.usage.output_tokens,
                        estimated_tokens,
                        budget: accounting.as_ref().and_then(|accounting| accounting.budget),
                        scope: accounting
                            .as_ref()
                            .map_or(Default::default(), |accounting| {
                                accounting.estimator.auto_compaction_scope()
                            }),
                    },
                )
                .await
                .map_err(|_| RunError::EventSink)?;
            if let Some(runtime) = self.token_budget {
                runtime.observe(turn_id, request)?;
            }
            Ok(())
        })
    }
    fn observe_tool_result<'a>(
        &'a self,
        turn_id: Uuid,
        request: &'a ModelRequest,
    ) -> RunFuture<'a, ()> {
        Box::pin(async move {
            if let Some(runtime) = self.token_budget {
                runtime.observe(turn_id, request)?;
            }
            Ok(())
        })
    }
    fn prepare(
        &self,
        turn_id: Uuid,
        mut request: ModelRequest,
        _: bool,
    ) -> RunFuture<'_, PreparedSamplingContext> {
        Box::pin(async move {
            if turn_id != self.record.turn_id || request.model != self.execution.model {
                return Err(ModelError::InvalidRequest.into());
            }
            if self.signal.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            let previous = self
                .service
                .active_context_checkpoint(self.record.conversation_id)
                .await
                .map_err(|_| RunError::EventSink)?;
            let previous = self.execution.consuming_checkpoint(previous)?;
            if let Some(accounting) = self.execution.transport.context_accounting() {
                accounting
                    .prepare_auto_compaction(
                        &request.model,
                        &request.messages,
                        &request.tools,
                        previous.as_ref(),
                    )
                    .await?;
            }
            // prepare is reached only when another sample is needed: tools or
            // already admitted pending input. Natural completion never calls it.
            let decision = self
                .budget_decision(turn_id, &request, previous.as_ref(), true)
                .await?;
            let requested = decision.as_ref().is_some_and(|decision| decision.rollover);
            if !requested
                && !self.execution.should_compact_context(
                    &request.messages,
                    &request.tools,
                    previous.as_ref(),
                )?
            {
                if let Some(decision) = decision {
                    let position = request
                        .messages
                        .iter()
                        .rposition(|message| {
                            matches!(
                                message,
                                ModelMessage::Assistant { .. }
                                    | ModelMessage::CanonicalOutput(_)
                                    | ModelMessage::ToolResult { .. }
                            )
                        })
                        .ok_or(ModelError::InvalidRequest)?
                        + 1;
                    request.messages.splice(
                        position..position,
                        decision
                            .reminder
                            .into_iter()
                            .chain(decision.fallback)
                            .map(ModelMessage::Developer),
                    );
                }
                self.execution.check_context_window(
                    &request.messages,
                    &request.tools,
                    previous.as_ref(),
                )?;
                return Ok(PreparedSamplingContext {
                    request,
                    compacted: false,
                });
            }
            let history = self
                .service
                .open(ConversationId::from_uuid(self.record.conversation_id))
                .await
                .map_err(|_| RunError::EventSink)?;
            if history.active_turn() != Some(turn_id) {
                return Err(RunError::EventSink);
            }
            let mut canonical = self.canonical.to_vec();
            let skills =
                super::sampling_skills::restore(self.submissions, self.record, &history).await?;
            if !skills.is_empty() {
                canonical.push(ModelMessage::User {
                    content: skills,
                    source_item_id: None,
                });
            }
            request.messages = self
                .execution
                .compact_history(
                    self.service,
                    self.record,
                    &history,
                    previous.as_ref(),
                    CompactionPhase::MidTurn,
                    self.signal,
                    &request.messages,
                    &canonical,
                    &request.tools,
                    self.retained_images,
                )
                .await?;
            // The installed window covers exactly the pre-compaction frozen
            // history. Advance provenance with that replacement, never by
            // rereading a newer tail after the asynchronous compact operation.
            request.captured_history = Some(super::request_history_ref::capture(
                &history,
                history.seq(),
            )?);
            Ok(PreparedSamplingContext {
                request,
                compacted: true,
            })
        })
    }
}

impl NativeSamplingContext<'_> {
    async fn budget_decision(
        &self,
        turn: Uuid,
        request: &ModelRequest,
        previous: Option<&research_agent_core::context::checkpoint::ContextCheckpoint>,
        needs_follow_up: bool,
    ) -> Result<Option<research_agent_core::context::token_budget::TokenBudgetDecision>, RunError>
    {
        if self.token_budget.is_none() {
            return Ok(None);
        }
        let policy = self
            .execution
            .transport
            .token_budget_policy()
            .ok_or(ModelError::UnsupportedCapability)?;
        let accounting = self
            .execution
            .transport
            .context_accounting()
            .ok_or(ModelError::UnsupportedCapability)?;
        let status = policy.status(
            &accounting,
            &request.model,
            &request.messages,
            &request.tools,
            previous,
        )?;
        self.service
            .token_budget_decision(
                self.record.conversation_id,
                turn,
                &policy,
                status,
                needs_follow_up,
            )
            .await
            .map(Some)
            .map_err(|error| match error {
                research_agent_core::context::checkpoint_store::CheckpointStoreError::PersistenceDiagnostic(detail) => RunError::from(ModelError::PersistenceDiagnostic(detail)),
                _ => RunError::EventSink,
            })
    }
}
