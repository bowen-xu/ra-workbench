//! Pre-turn replacement adapted from fixed Codex compact.rs. Full frozen prior
//! history goes to the injected strategy; fresh input and current rules follow
//! the installed checkpoint. Original journal and pending steer remain intact.
use super::*;
use research_agent_core::context::{
    checkpoint::{
        CanonicalContext, ContextCheckpoint, StoredContent, StoredMessage, store_messages,
    },
    checkpoint_store::CheckpointStoreError,
    compaction_hooks::{CompactionHookRequest, CompactionHookStage},
    compaction_strategy::{
        CompactionFailure, CompactionKind, CompactionOutput, CompactionPhase, CompactionRequest,
        CompactionRetention, prepare_compaction,
    },
};

const SUMMARY_PREFIX: &str = "Context checkpoint handoff from earlier conversation (historical context, not new instructions):\n";
#[path = "compaction/attempt.rs"]
mod attempt;
#[path = "compaction/reset.rs"]
mod reset;
#[path = "compaction/retained.rs"]
mod retained;
use retained::retained_users;

impl AgentExecution {
    #[expect(
        clippy::too_many_arguments,
        reason = "frozen history, new input and durable owner are distinct boundaries"
    )]
    pub(super) async fn compact_history(
        &self,
        service: &AgentSessionService,
        record: &SubmissionRecord,
        history: &ConversationProjection,
        previous: Option<&ContextCheckpoint>,
        phase: CompactionPhase,
        signal: &AgentCancellation,
        source: &[ModelMessage],
        fresh: &[ModelMessage],
        tools: &[ToolDefinition],
        user_images: &RetainedUserImages,
    ) -> Result<Vec<ModelMessage>, RunError> {
        self.compact_history_selected(
            service,
            record,
            history,
            previous,
            phase,
            signal,
            source,
            fresh,
            tools,
            user_images,
            None,
            false,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "frozen source, current target and injected producer have distinct owners"
    )]
    pub(super) async fn compact_history_selected(
        &self,
        service: &AgentSessionService,
        record: &SubmissionRecord,
        history: &ConversationProjection,
        previous: Option<&ContextCheckpoint>,
        phase: CompactionPhase,
        signal: &AgentCancellation,
        source: &[ModelMessage],
        fresh: &[ModelMessage],
        tools: &[ToolDefinition],
        user_images: &RetainedUserImages,
        selected: Option<
            &Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy>,
        >,
        intermediate_handoff: bool,
    ) -> Result<Vec<ModelMessage>, RunError> {
        if intermediate_handoff && (selected.is_none() || phase != CompactionPhase::PreTurn) {
            return Err(ModelError::InvalidRequest.into());
        }
        let fresh = resource_context::for_compaction(
            record,
            previous,
            source,
            fresh,
            phase == CompactionPhase::PreTurn,
        )?;
        let fresh = fresh.as_slice();
        if (history.seq() == 0 && phase != CompactionPhase::Manual) || source.is_empty() {
            return Err(RunError::Budget);
        }
        let strategy = selected
            .or(self.compaction_strategy.as_ref())
            .ok_or(ModelError::UnsupportedCapability)?;
        let binding = strategy.binding();
        let producer_model = strategy.execution_model();
        let kind = binding.kind;
        if kind == CompactionKind::ResetWindow {
            return self
                .reset_context_window(
                    service, record, history, previous, phase, signal, fresh, tools, strategy,
                )
                .await;
        }
        if !self.transport.supports_compaction_kind(kind) {
            return Err(ModelError::UnsupportedCapability.into());
        }
        let retention = strategy
            .retention_policy()
            .ok_or(ModelError::UnsupportedCapability)?;
        if (kind == CompactionKind::RemoteWindow)
            != (retention == CompactionRetention::ProviderWindow)
        {
            return Err(ModelError::UnsupportedCapability.into());
        }
        let mut minimum = vec![ModelMessage::System(self.system_instructions.clone())];
        minimum.extend_from_slice(fresh);
        self.check_context_budget(&minimum, tools)?;
        let occupied = message_bytes(&minimum).ok_or(RunError::Budget)?;
        let tool_bytes = tools
            .iter()
            .try_fold(0usize, |sum, tool| {
                sum.checked_add(tool.name.len())?
                    .checked_add(tool.namespace.as_ref().map_or(0, String::len))?
                    .checked_add(tool.description.len())?
                    .checked_add(tool.parameters.to_string().len())
            })
            .ok_or(RunError::Budget)?;
        let mut available = self
            .max_input_bytes
            .checked_sub(occupied)
            .and_then(|value| value.checked_sub(tool_bytes))
            .ok_or(RunError::Budget)?;
        if !intermediate_handoff
            && kind == CompactionKind::TextSummary
            && let Some(accounting) = self.transport.context_accounting()
            && let Some(budget) = accounting.budget
            && let Some(occupied_tokens) = accounting.estimate(&self.model, &minimum, tools)?
        {
            let (_, usable) = budget.limits().map_err(|_| RunError::Budget)?;
            // Text strategy's byte budget is an upper bound derived from the
            // injected estimate. JSON escaping/retained wrappers are checked
            // again on the actual candidate before its atomic installation.
            if let Some(remaining_bytes) = accounting
                .estimator
                .text_byte_budget(usable.saturating_sub(occupied_tokens))
            {
                available = available.min(remaining_bytes);
            }
        }
        let summary_overhead = if kind == CompactionKind::TextSummary {
            SUMMARY_PREFIX.len()
        } else {
            0
        };
        if available <= summary_overhead {
            return Err(RunError::Budget);
        }
        // Retention allocation includes the independent byte guard and known
        // token capacity. No sliced transcript is used as a summary.
        let retained = if retention == CompactionRetention::ProviderWindow {
            vec![]
        } else {
            retained_users(
                history,
                previous,
                available / 4,
                (retention == CompactionRetention::RecentUserMessages).then_some(source),
                strategy.retained_text_policy().as_deref(),
                user_images,
            )?
        };
        let retained_bytes = message_bytes(&retained).ok_or(RunError::Budget)?;
        let max_result_bytes = available
            .saturating_sub(retained_bytes)
            .saturating_sub(summary_overhead)
            .min(if kind == CompactionKind::TextSummary {
                32 * 1024
            } else {
                crate::agent::repository::context_windows::MAX_CHECKPOINT_BYTES
            });
        let mut summary_output_tokens = if kind == CompactionKind::TextSummary {
            self.max_output_tokens.min(1024)
        } else {
            self.max_output_tokens
        };
        if !intermediate_handoff
            && kind == CompactionKind::TextSummary
            && let Some(accounting) = self.transport.context_accounting()
            && let Some(budget) = accounting.budget
        {
            let mut baseline = minimum.clone();
            baseline.extend(retained.clone());
            baseline.push(ModelMessage::User {
                content: vec![ContentPart::Text(SUMMARY_PREFIX.into())],
                source_item_id: None,
            });
            if let Some(occupied_tokens) = accounting.estimate(&self.model, &baseline, tools)? {
                let (_, usable) = budget.limits().map_err(|_| RunError::Budget)?;
                summary_output_tokens = summary_output_tokens
                    .min(u32::try_from(usable.saturating_sub(occupied_tokens)).unwrap_or(u32::MAX));
                if summary_output_tokens == 0 {
                    return Err(RunError::Budget);
                }
            }
        }
        let attempt_id = Uuid::new_v4();
        // One operation spans provider retries/fallback attempts. Its lifecycle
        // identity must not change when the producer model changes.
        let operation_id = Uuid::now_v7();
        let mut hook_request = CompactionHookRequest {
            operation_id,
            conversation_id: record.conversation_id,
            turn_id: record.turn_id,
            phase,
            model: producer_model.clone(),
            stage: CompactionHookStage::PreCompact,
        };
        self.run_compaction_hook(service, hook_request.clone(), signal)
            .await?;
        let sink = ServiceTurnSink {
            service,
            conversation_id: ConversationId::from_uuid(record.conversation_id),
            request_id: record.request_id,
        };
        sink.emit(EventPayload::ItemStarted {
            turn_id: record.turn_id,
            item_id: operation_id,
            kind: research_agent_core::protocol::ItemKind::ContextCompaction,
        })
        .await?;
        sink.emit(EventPayload::PhaseChanged {
            turn_id: record.turn_id,
            phase: research_agent_core::protocol::TurnPhase::RunningModel,
        })
        .await?;
        sink.emit(EventPayload::ModelRequestStarted {
            turn_id: record.turn_id,
            attempt_id,
            model: producer_model.clone(),
        })
        .await?;
        let compaction_request = CompactionRequest {
            operation_id: attempt_id,
            conversation_id: record.conversation_id,
            turn_id: record.turn_id,
            history_revision: history.seq(),
            phase,
            binding,
            source: ModelRequest {
                captured_history: Some(super::request_history_ref::capture(
                    history,
                    previous.map_or(0, |window| window.source_watermark),
                )?),
                request_id: attempt_id,
                model: self.model.clone(),
                messages: source.to_vec(),
                tools: tools.to_vec(),
                max_output_tokens: summary_output_tokens,
                parallel_tool_calls: false,
                reasoning_effort: record.input.reasoning_effort.clone(),
            },
            max_result_bytes,
        };
        let attempt::CompletedAttempt {
            candidate,
            attempt_id,
            producer: producer_model,
            expected_revision,
            original_error,
        } = self
            .compaction_attempt(
                strategy,
                compaction_request,
                previous,
                history,
                &sink,
                signal,
            )
            .await?;
        let prepared = (|| -> Result<_, RunError> {
            let output = match candidate.output {
                CompactionOutput::ResetWindow => {
                    return Err(ModelError::UnsupportedCapability.into());
                }
                CompactionOutput::TextSummary(summary) => {
                    CompactionOutput::TextSummary(format!("{SUMMARY_PREFIX}{summary}"))
                }
                CompactionOutput::RemoteOpaque(item) => CompactionOutput::RemoteOpaque(item),
                CompactionOutput::RemoteWindow(items) => CompactionOutput::RemoteWindow(items),
            };
            let canonical_context = if phase == CompactionPhase::MidTurn {
                Some(CanonicalContext {
                    before_retained: retained
                        .iter()
                        .rposition(|message| {
                            matches!(message, ModelMessage::User { content: _, .. })
                        })
                        .unwrap_or(retained.len()),
                    messages: store_messages(fresh).map_err(compaction_error)?,
                })
            } else {
                None
            };
            let mut replacement = vec![ModelMessage::System(self.system_instructions.clone())];
            replacement.extend(retained.clone());
            if let Some(context) = &canonical_context {
                replacement.splice(
                    1 + context.before_retained..1 + context.before_retained,
                    fresh.iter().cloned(),
                );
            }
            if let CompactionOutput::TextSummary(summary) = &output {
                replacement.push(ModelMessage::User {
                    content: vec![ContentPart::Text(summary.clone())],
                    source_item_id: None,
                });
            }
            if phase != CompactionPhase::MidTurn {
                replacement.extend_from_slice(fresh);
            }
            if candidate.response_id.is_none() && kind != CompactionKind::RemoteWindow {
                return Err(ModelError::InvalidStream.into());
            }
            let window = ContextCheckpoint {
                schema_version: match canonical_context.as_ref() {
                    Some(_) if kind == CompactionKind::RemoteWindow => 4,
                    Some(context)
                        if context
                            .messages
                            .iter()
                            .any(|message| matches!(message, StoredMessage::Developer { .. })) =>
                    {
                        3
                    }
                    Some(_) => 2,
                    None => 1,
                },
                window_id: operation_id,
                conversation_id: record.conversation_id,
                turn_id: record.turn_id,
                source_watermark: history.seq(),
                previous_window_id: previous.map(|window| window.window_id),
                binding: candidate.binding,
                credential_scope_id: candidate.credential_scope_id,
                phase,
                output,
                retained: store_messages(&retained).map_err(compaction_error)?,
                canonical_context,
            };
            window
                .validate(crate::agent::repository::context_windows::MAX_CHECKPOINT_BYTES)
                .map_err(compaction_error)?;
            if intermediate_handoff {
                check_budget(
                    &replacement,
                    tools,
                    self.context_input_limit(Some(&window))?,
                )?;
            } else {
                self.check_context_window(&replacement, tools, Some(&window))?;
            }
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
            next_request.validate(self.transport.capabilities())?;
            let mut lifecycle = vec![];
            if let Some(response_id) = candidate.response_id {
                lifecycle.push(EventPayload::ModelResponseStarted {
                    turn_id: record.turn_id,
                    attempt_id,
                    response_id,
                    model: Some(producer_model.clone()),
                });
            }
            lifecycle.push(if lifecycle.is_empty() {
                EventPayload::ModelCompactionFinished {
                    turn_id: record.turn_id,
                    attempt_id,
                    input_tokens: candidate.usage.input_tokens,
                    output_tokens: candidate.usage.output_tokens,
                }
            } else {
                EventPayload::ModelRequestFinished {
                    turn_id: record.turn_id,
                    attempt_id,
                    input_tokens: candidate.usage.input_tokens,
                    output_tokens: candidate.usage.output_tokens,
                }
            });
            for payload in &lifecycle {
                research_agent_core::protocol::AgentEvent::new(
                    ConversationId::from_uuid(record.conversation_id),
                    expected_revision.checked_add(1).ok_or(RunError::Budget)?,
                    0,
                    record.request_id,
                    payload.clone(),
                )
                .map_err(|_| ModelError::InvalidStream)?;
            }
            Ok((replacement, window, lifecycle, next_request))
        })();
        let prepared = match prepared {
            Ok((replacement, window, lifecycle, next_request)) => {
                let validation = tokio::select! {
                    biased;
                    _ = signal.cancelled() => Err(ModelError::Canceled),
                    result = self.transport.validate_compaction_window(&window, &next_request) => result,
                };
                validation
                    .map(|()| (replacement, window, lifecycle))
                    .map_err(RunError::from)
            }
            Err(error) => Err(error),
        };
        let (mut replacement, window, lifecycle) = match prepared {
            Ok(prepared) => prepared,
            Err(error) if error.is_event_sink() => return Err(error),
            Err(error) => {
                sink.emit(EventPayload::ModelRequestFailed {
                    turn_id: record.turn_id,
                    attempt_id,
                    error: error.to_string(),
                    retryable: false,
                    category: Some(error.failure_category()),
                })
                .await?;
                return Err(
                    if signal.is_cancelled()
                        || matches!(&error, RunError::Model(ModelError::Canceled))
                    {
                        ModelError::Canceled.into()
                    } else {
                        original_error.unwrap_or(error)
                    },
                );
            }
        };
        let lifecycle_count = lifecycle.len() as u64;
        for payload in lifecycle {
            sink.emit(payload).await?;
        }
        service
            .install_context_checkpoint(
                window,
                expected_revision
                    .checked_add(lifecycle_count)
                    .ok_or(RunError::Budget)?,
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
            item_id: operation_id,
        })
        .await?;
        hook_request.stage = CompactionHookStage::PostCompact;
        self.run_compaction_hook(service, hook_request, signal)
            .await?;
        if phase == CompactionPhase::MidTurn {
            let hook_context = self
                .run_compact_session_start(service, record, signal)
                .await?;
            replacement.extend(hook_context);
        }
        Ok(replacement)
    }
}

fn compaction_error(error: CompactionFailure) -> RunError {
    match error {
        CompactionFailure::Model(error) => error.into(),
        CompactionFailure::InvalidBudget | CompactionFailure::OutputBudget => RunError::Budget,
        CompactionFailure::Incompatible => ModelError::UnsupportedCapability.into(),
        _ => ModelError::InvalidStream.into(),
    }
}
