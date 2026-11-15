//! No-model reset under the existing task owner, adapted from Codex
//! compact_token_budget.rs (Apache-2.0), e363b08c9175ac1cbe5893615dd2cb9ddf95043b.
//! Copyright 2025 OpenAI. Candidate installation keeps the application's durable
//! CAS/commit gate, rather than upstream's memory-first replacement.
use super::*;
use research_agent_core::context::compaction_strategy::CompactionStrategy;

impl AgentExecution {
    #[expect(
        clippy::too_many_arguments,
        reason = "captured context, durable owner and cancellation are separate boundaries"
    )]
    pub(in crate::agent::execution) async fn reset_context_window(
        &self,
        service: &AgentSessionService,
        record: &SubmissionRecord,
        history: &ConversationProjection,
        previous: Option<&ContextCheckpoint>,
        phase: CompactionPhase,
        signal: &AgentCancellation,
        fresh: &[ModelMessage],
        tools: &[ToolDefinition],
        strategy: &Arc<dyn CompactionStrategy>,
    ) -> Result<Vec<ModelMessage>, RunError> {
        if strategy.binding().kind != CompactionKind::ResetWindow
            || strategy.retention_policy() != Some(CompactionRetention::ResetWindow)
            || strategy.binding().model != self.model
            || strategy.execution_model() != self.model
        {
            return Err(ModelError::UnsupportedCapability.into());
        }
        // Pre-turn fresh ends in the new real user input. Preserve it for the
        // next sample, but do not freeze it as canonical context in the checkpoint.
        let (canonical, current_input) = if phase == CompactionPhase::PreTurn {
            let (input, canonical) = fresh.split_last().ok_or(ModelError::InvalidRequest)?;
            if !matches!(input, ModelMessage::User { content: _, .. }) {
                return Err(ModelError::InvalidRequest.into());
            }
            (canonical, Some(input))
        } else {
            (fresh, None)
        };
        let mut canonical = canonical.to_vec();
        let window_id = Uuid::now_v7();
        let mut hook = CompactionHookRequest {
            operation_id: window_id,
            conversation_id: record.conversation_id,
            turn_id: record.turn_id,
            phase,
            model: self.model.clone(),
            stage: CompactionHookStage::PreCompact,
        };
        self.run_compaction_hook(service, hook.clone(), signal)
            .await?;
        if self.transport.token_budget_policy().is_some()
            && self
                .transport
                .context_accounting()
                .is_some_and(|accounting| accounting.budget.is_some())
        {
            let identity = service
                .prepare_token_budget_window(record.conversation_id, record.turn_id)
                .await
                .map_err(|_| RunError::EventSink)?;
            let thread_hint = tokio::select! {
                biased;
                _ = signal.cancelled() => return Err(ModelError::Canceled.into()),
                hint = service.thread_hint(record.conversation_id) => hint,
            };
            canonical.push(
                research_agent_core::context::token_budget::TokenBudgetContext {
                    conversation_id: record.conversation_id,
                    first_window_id: identity.first,
                    previous_window_id: Some(identity.current),
                    window_id,
                    thread_hint,
                }
                .into_message(),
            );
        }
        if let Some(guidance) = self
            .transport
            .token_budget_policy()
            .and_then(|policy| policy.guidance_fragment())
        {
            canonical.push(ModelMessage::Developer(guidance));
        }
        let candidate = tokio::select! {
                    biased;
                    _ = signal.cancelled() => return Err(ModelError::Canceled.into()),
                    candidate = prepare_compaction(strategy.as_ref(), CompactionRequest {
                        operation_id: window_id,
                        conversation_id: record.conversation_id,
                        turn_id: record.turn_id,
                        history_revision: history.seq(), phase,
                        binding: strategy.binding(),
                        source: ModelRequest {
        captured_history: None,
                            request_id: window_id, model: self.model.clone(),
                            messages: vec![], tools: vec![], max_output_tokens: self.max_output_tokens,
                            parallel_tool_calls: false, reasoning_effort: record.input.reasoning_effort.clone(),
                        },
                        max_result_bytes: crate::agent::repository::context_windows::MAX_CHECKPOINT_BYTES,
                    }) => candidate.map_err(compaction_error)?,
                };
        if candidate.response_id.is_some() || candidate.usage != ModelUsage::default() {
            return Err(ModelError::InvalidStream.into());
        }
        let window = ContextCheckpoint {
            schema_version: if canonical
                .iter()
                .any(|message| matches!(message, ModelMessage::Developer(_)))
            {
                3
            } else {
                2
            },
            window_id,
            conversation_id: record.conversation_id,
            turn_id: record.turn_id,
            source_watermark: history.seq(),
            previous_window_id: previous.map(|window| window.window_id),
            binding: candidate.binding,
            credential_scope_id: candidate.credential_scope_id,
            phase,
            output: candidate.output,
            retained: vec![],
            canonical_context: Some(CanonicalContext {
                before_retained: 0,
                messages: store_messages(&canonical).map_err(compaction_error)?,
            }),
        };
        window
            .validate(crate::agent::repository::context_windows::MAX_CHECKPOINT_BYTES)
            .map_err(compaction_error)?;
        let mut replacement = vec![ModelMessage::System(self.system_instructions.clone())];
        replacement.extend_from_slice(&canonical);
        replacement.extend(current_input.cloned());
        self.check_context_window(&replacement, tools, Some(&window))?;
        let next_request = ModelRequest {
            captured_history: None,
            request_id: Uuid::new_v4(),
            model: self.model.clone(),
            messages: replacement.clone(),
            tools: tools.to_vec(),
            max_output_tokens: self.max_output_tokens,
            parallel_tool_calls: false,
            reasoning_effort: record.input.reasoning_effort.clone(),
        };
        tokio::select! {
            biased;
            _ = signal.cancelled() => return Err(ModelError::Canceled.into()),
            result = self.transport.validate_compaction_window(&window, &next_request) => result?,
        }
        let sink = ServiceTurnSink {
            service,
            conversation_id: ConversationId::from_uuid(record.conversation_id),
            request_id: record.request_id,
        };
        // Admission events are already durable but outside the supplied source
        // history snapshot. Coverage and installation CAS are distinct revisions.
        let expected_revision = service
            .open(ConversationId::from_uuid(record.conversation_id))
            .await
            .map_err(|_| RunError::EventSink)?
            .seq();
        sink.emit(EventPayload::ItemStarted {
            turn_id: record.turn_id,
            item_id: window_id,
            kind: research_agent_core::protocol::ItemKind::ContextCompaction,
        })
        .await?;
        service
            .install_context_checkpoint(
                window,
                expected_revision.checked_add(1).ok_or(RunError::Budget)?,
                signal.clone(),
            )
            .await
            .map_err(|error| match error {
                CheckpointStoreError::Canceled => RunError::Model(ModelError::Canceled),
                CheckpointStoreError::PersistenceDiagnostic(detail) => {
                    RunError::from(ModelError::PersistenceDiagnostic(detail))
                }
                _ => RunError::EventSink,
            })?;
        sink.emit(EventPayload::ItemCompleted {
            turn_id: record.turn_id,
            item_id: window_id,
        })
        .await?;
        hook.stage = CompactionHookStage::PostCompact;
        self.run_compaction_hook(service, hook, signal).await?;
        if phase == CompactionPhase::MidTurn {
            let hook_context = self
                .run_compact_session_start(service, record, signal)
                .await?;
            replacement.extend(hook_context);
        }
        Ok(replacement)
    }
}
