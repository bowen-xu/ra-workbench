//! Build a model request from current authority and durable conversation history.
use super::*;

impl AgentExecution {
    pub(super) async fn request(
        &self,
        service: &AgentSessionService,
        submissions: &dyn SubmissionStore,
        record: &SubmissionRecord,
        history: &ConversationProjection,
        preparation: RequestPreparation<'_>,
    ) -> Result<ModelRequest, RunError> {
        let RequestPreparation {
            compact_only,
            signal,
            materializer,
            tool_definitions,
            canonical_out,
            retained_images_out,
        } = preparation;
        if signal.is_cancelled() {
            return Err(ModelError::Canceled.into());
        }
        if record.input.model != self.model {
            return Err(ModelError::InvalidRequest.into());
        }
        tokio::select! {
            biased;
            _ = signal.cancelled() => return Err(ModelError::Canceled.into()),
            result = self.transport.prepare_context_policy() => result?,
        }
        let stored_checkpoint = service
            .active_context_checkpoint(record.conversation_id)
            .await
            .map_err(|_| ModelError::Unavailable)?;
        if compact_only
            && let Some(strategy) = self.compaction_strategy.as_ref()
            && strategy.binding().kind
                == research_agent_core::context::compaction_strategy::CompactionKind::ResetWindow
        {
            let captured_history =
                Some(super::request_history_ref::capture(history, history.seq())?);
            // An explicit reset discards the former window. Its opaque codec,
            // credentials and historical resources are not reset dependencies.
            return self
                .reset_context_window(
                    service,
                    record,
                    history,
                    stored_checkpoint.as_ref(),
                    research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
                    signal,
                    &[],
                    tool_definitions,
                    strategy,
                )
                .await
                .map(|messages| ModelRequest {
                    captured_history,
                    request_id: record.request_id,
                    model: self.model.clone(),
                    messages,
                    tools: tool_definitions.to_vec(),
                    max_output_tokens: self.max_output_tokens,
                    parallel_tool_calls: false,
                    reasoning_effort: record.input.reasoning_effort.clone(),
                });
        }
        if let Some(checkpoint) = &stored_checkpoint {
            let strategy = self
                .compaction_strategy
                .as_ref()
                .ok_or(ModelError::UnsupportedCapability)?;
            if !strategy.accepts_binding(&checkpoint.binding) {
                return Err(ModelError::UnsupportedCapability.into());
            }
            strategy
                .validate_stored_output(&checkpoint.binding, &checkpoint.output)
                .map_err(|_| ModelError::InvalidStream)?;
        }
        // Explicit strategy compatibility grants only a transient consuming view.
        // The journal/checkpoint keeps its original production identity.
        let checkpoint = self.consuming_checkpoint(stored_checkpoint.clone())?;
        let mut hook_prefixes: HashMap<Uuid, Vec<String>> = HashMap::new();
        for batch in service
            .compact_hook_contexts(record.conversation_id)
            .await
            .map_err(|_| RunError::EventSink)?
        {
            if batch.turn_id != record.turn_id && history.turn(batch.turn_id).is_some() {
                hook_prefixes
                    .entry(batch.turn_id)
                    .or_default()
                    .extend(batch.contexts);
            }
        }
        let tail = checkpoint
            .as_ref()
            .map(|window| history.context_tail(window.source_watermark))
            .transpose()
            .map_err(|_| ModelError::InvalidRequest)?;
        let retained_image_hashes: Vec<String> =
            checkpoint.as_ref().map_or_else(Vec::new, |window| {
                use research_agent_core::context::checkpoint::{StoredContent, StoredMessage};
                window
                    .retained
                    .iter()
                    .chain(
                        window
                            .canonical_context
                            .iter()
                            .flat_map(|context| &context.messages),
                    )
                    .flat_map(|message| match message {
                        StoredMessage::User { content, .. }
                        | StoredMessage::Assistant { content, .. } => content.as_slice(),
                        StoredMessage::Developer { .. }
                        | StoredMessage::ToolResult { .. }
                        | StoredMessage::CanonicalOutput { .. } => &[],
                    })
                    .filter_map(|part| match part {
                        StoredContent::Image { sha256, .. } => Some(sha256.clone()),
                        _ => None,
                    })
                    .collect()
            });
        let needs_retained_images = !retained_image_hashes.is_empty();
        let mut contexts = HashMap::new();
        let mut user_contexts = HashMap::new();
        let mut resource_scope =
            resource_context::ResourceContext::from_checkpoint(checkpoint.as_ref());
        for turn_id in history.turn_order() {
            if history
                .turn(*turn_id)
                .is_some_and(|turn| !turn.task_kind.is_regular())
            {
                continue;
            }
            let needs_turn_context = checkpoint
                .as_ref()
                .is_none_or(|window| history.turn_started_after(*turn_id, window.source_watermark));
            let needs_tail_user_context = tail.as_ref().is_some_and(|tail| {
                tail.turn(*turn_id).is_some_and(|turn| {
                    turn.items.values().any(|item| {
                        matches!(
                            item.kind,
                            research_agent_core::protocol::ItemKind::UserMessage
                        )
                    })
                })
            });
            // Covered sources and images are not dependencies of the new request.
            // Same-turn steer may still need its frozen instruction identities.
            if !needs_turn_context && !needs_tail_user_context && !needs_retained_images {
                continue;
            }
            if signal.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            let previous = submissions
                .get_history_input(record.conversation_id, *turn_id)
                .await
                .map_err(|_| ModelError::InvalidRequest)?;
            let turn = history.turn(*turn_id).ok_or(ModelError::InvalidRequest)?;
            if previous.conversation_id != record.conversation_id
                || previous.turn_id != *turn_id
                || previous.context_snapshot_id != turn.context_snapshot_id
                || previous.input.model != turn.model
            {
                return Err(ModelError::InvalidRequest.into());
            }
            let parts = if needs_turn_context {
                incremental_context_parts(previous.turn_id, &previous.input, &mut resource_scope)?
            } else {
                vec![]
            };
            let mut injected: std::collections::HashSet<_> = previous
                .input
                .skill_instructions
                .iter()
                .map(|prompt| prompt.identity.clone())
                .collect();
            for input in submissions
                .get_history_steering(record.conversation_id, *turn_id)
                .await
                .map_err(|_| ModelError::InvalidRequest)?
            {
                if turn.items.get(&input.input_id).is_none_or(|item| {
                    !item.completed
                        || !matches!(
                            item.kind,
                            research_agent_core::protocol::ItemKind::UserMessage
                        )
                        || item.text != input.text
                }) {
                    continue;
                }
                let mut additions: Vec<_> = input
                    .skill_instructions
                    .iter()
                    .filter(|prompt| injected.insert(prompt.identity.clone()))
                    .map(|prompt| ContentPart::Text(prompt.render()))
                    .collect();
                additions.extend(
                    evidence_parts(
                        &input.user_input,
                        *turn_id,
                        retained_images_out,
                        submissions,
                    )
                    .await?,
                );
                user_contexts.insert(input.input_id, additions);
            }
            let evidence = evidence_parts(
                &previous.input.user_input,
                *turn_id,
                retained_images_out,
                submissions,
            )
            .await?;
            if !evidence.is_empty() {
                let item_id = original_user_item(history, *turn_id)?;
                if turn.items[&item_id].text != previous.input.text {
                    return Err(ModelError::InvalidRequest.into());
                }
                if needs_turn_context || needs_tail_user_context {
                    user_contexts
                        .entry(item_id)
                        .or_insert_with(Vec::new)
                        .extend(evidence);
                }
            }
            if (needs_turn_context || needs_retained_images)
                && let Some(materializer) = materializer
            {
                let images = if needs_turn_context {
                    materializer
                        .materialize_history(record.conversation_id, *turn_id)
                        .await
                } else {
                    materializer
                        .materialize_history_images(
                            record.conversation_id,
                            *turn_id,
                            retained_image_hashes.clone(),
                        )
                        .await
                }
                .map_err(image_error)?;
                if !images.is_empty() {
                    let images = image_parts(&images)?;
                    retained_images_out
                        .entry(*turn_id)
                        .or_default()
                        .extend(images.clone());
                    let item_id = original_user_item(history, *turn_id)?;
                    if turn.items[&item_id].text != previous.input.text {
                        return Err(ModelError::InvalidRequest.into());
                    }
                    user_contexts
                        .entry(item_id)
                        .or_insert_with(Vec::new)
                        .extend(images);
                }
            }
            contexts.insert(*turn_id, parts);
        }
        let fragments = service
            .context_fragments(
                record.conversation_id,
                stored_checkpoint
                    .as_ref()
                    .map(|checkpoint| checkpoint.window_id),
                stored_checkpoint
                    .as_ref()
                    .map_or(0, |checkpoint| checkpoint.source_watermark),
            )
            .await?;
        let mut messages = vec![ModelMessage::System(self.system_instructions.clone())];
        if let Some(checkpoint) = &checkpoint {
            use research_agent_core::context::compaction_strategy::CompactionOutput;
            let images: HashMap<_, _> = contexts
                .values()
                .chain(retained_images_out.values())
                .flatten()
                .filter_map(|part| match part {
                    ContentPart::Image { bytes, .. } => {
                        use sha2::{Digest, Sha256};
                        Some((format!("{:x}", Sha256::digest(bytes)), bytes.clone()))
                    }
                    _ => None,
                })
                .collect();
            let resolve = |hash: &str| images.get(hash).cloned();
            messages.extend(
                match &checkpoint.output {
                    CompactionOutput::ResetWindow => checkpoint.restore_reset_window(&resolve),
                    CompactionOutput::TextSummary(_) => checkpoint.restore_text_window(&resolve),
                    CompactionOutput::RemoteOpaque(_) => {
                        checkpoint.restore_opaque_messages(&resolve)
                    }
                    CompactionOutput::RemoteWindow(_) => {
                        checkpoint.restore_provider_window_tail(&resolve)
                    }
                }
                .map_err(|_| ModelError::InvalidRequest)?,
            );
            messages.extend(
                research_agent_core::model::reconstruct_history_with_developer_context(
                    history,
                    &contexts,
                    &user_contexts,
                    &fragments,
                    &hook_prefixes,
                    Some(checkpoint.source_watermark),
                )
                .map_err(|_| ModelError::InvalidRequest)?,
            );
        } else {
            if self.transport.token_budget_policy().is_some()
                && self
                    .transport
                    .context_accounting()
                    .is_some_and(|accounting| accounting.budget.is_some())
            {
                let metadata = service.initial_window_metadata(record.conversation_id,record.turn_id,signal).await.map_err(|error| match error {
                    research_agent_core::context::checkpoint_store::CheckpointStoreError::Canceled => RunError::Model(ModelError::Canceled),
                    research_agent_core::context::checkpoint_store::CheckpointStoreError::PersistenceDiagnostic(detail) => RunError::from(ModelError::PersistenceDiagnostic(detail)),
                    _ => RunError::EventSink,
                })?;
                messages.push(ModelMessage::Developer(metadata));
            }
            if let Some(guidance) = self
                .transport
                .token_budget_policy()
                .and_then(|policy| policy.guidance_fragment())
            {
                messages.push(ModelMessage::Developer(guidance));
            }
            messages.extend(
                research_agent_core::model::reconstruct_history_with_developer_context(
                    history,
                    &contexts,
                    &user_contexts,
                    &fragments,
                    &hook_prefixes,
                    None,
                )
                .map_err(|_| ModelError::InvalidRequest)?,
            );
        }
        project_history_tool_results(
            &mut messages,
            history,
            self.transport.tool_output_policy(),
            self.limits.tool_output_bytes,
        )?;
        let historical_end = messages.len();
        if let Some(checkpoint) = &stored_checkpoint {
            let restored_request = ModelRequest {
                captured_history: Some(super::request_history_ref::capture(
                    history,
                    checkpoint.source_watermark,
                )?),
                request_id: Uuid::new_v4(),
                model: self.model.clone(),
                messages: messages.clone(),
                tools: tool_definitions.to_vec(),
                max_output_tokens: self.max_output_tokens,
                parallel_tool_calls: false,
                reasoning_effort: record.input.reasoning_effort.clone(),
            };
            tokio::select! {
                biased;
                _ = signal.cancelled() => return Err(ModelError::Canceled.into()),
                result = self.transport.validate_compaction_window(checkpoint, &restored_request) => result?,
            }
        }
        if compact_only {
            let messages = self
                .compact_history(
                    service,
                    record,
                    history,
                    checkpoint.as_ref(),
                    research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
                    signal,
                    &messages,
                    &[],
                    tool_definitions,
                    retained_images_out,
                )
                .await?;
            return Ok(ModelRequest {
                captured_history: Some(super::request_history_ref::capture(
                    history,
                    history.seq(),
                )?),
                request_id: Uuid::new_v4(),
                model: self.model.clone(),
                messages,
                tools: tool_definitions.to_vec(),
                max_output_tokens: self.max_output_tokens,
                parallel_tool_calls: false,
                reasoning_effort: record.input.reasoning_effort.clone(),
            });
        }
        let selected = if let Some(previous_turn) = history.turn_order().iter().rev().find(|id| {
            **id != record.turn_id
                && history
                    .turn(**id)
                    .is_some_and(|turn| turn.task_kind.is_regular())
        }) {
            let previous_model = &history
                .turn(*previous_turn)
                .ok_or(ModelError::InvalidRequest)?
                .model;
            let active = self
                .transport
                .context_accounting()
                .map(|accounting| {
                    accounting.estimate_window(
                        &self.model,
                        &messages,
                        tool_definitions,
                        checkpoint.as_ref(),
                    )
                })
                .transpose()?
                .flatten();
            tokio::select! {
                biased;
                _ = signal.cancelled() => return Err(ModelError::Canceled.into()),
                result = self.transport.pre_turn_compaction(*previous_turn,previous_model,active) => result?,
            }
        } else {
            None
        };
        // Full host context is retained for a future mid-turn replacement; the
        // ordinary request only adds a resource fragment when authority changed.
        if let Some(out) = canonical_out {
            let canonical = context_parts(record)?;
            if !canonical.is_empty() {
                out.push(ModelMessage::User {
                    content: canonical,
                    source_item_id: None,
                });
            }
        }
        let context =
            incremental_context_parts(record.turn_id, &record.input, &mut resource_scope)?;
        let mut user_content = evidence_parts(
            &record.input.user_input,
            record.turn_id,
            retained_images_out,
            submissions,
        )
        .await?;
        if let Some(materializer) = materializer {
            let images = materializer
                .materialize(record.conversation_id, record.request_id)
                .await
                .map_err(image_error)?;
            if !images.is_empty() {
                let images = image_parts(&images)?;
                retained_images_out
                    .entry(record.turn_id)
                    .or_default()
                    .extend(images.clone());
                user_content.extend(images);
            }
        }
        if !context.is_empty() {
            let message = ModelMessage::User {
                content: context,
                source_item_id: None,
            };
            messages.push(message);
        }
        if !record.input.text.is_empty() {
            user_content.push(ContentPart::Text(record.input.text.clone()));
        }
        if user_content.is_empty() {
            return Err(ModelError::InvalidRequest.into());
        }
        messages.push(ModelMessage::User {
            content: user_content,
            source_item_id: None,
        });
        let tool_definitions = tool_definitions.to_vec();
        let mut checkpoint = checkpoint;
        // Codex checks the existing window before the current turn's context
        // and user input are recorded; the full request is still checked below.
        if let Some(accounting) = self.transport.context_accounting() {
            accounting
                .prepare_auto_compaction(
                    &self.model,
                    &messages[..historical_end],
                    &tool_definitions,
                    checkpoint.as_ref(),
                )
                .await?;
        }
        // Match Codex: the pre-turn decision considers the existing window;
        // the fresh input is validated separately before sampling.
        if history.seq() > 0
            && (selected.is_some()
                || self.should_compact_context(
                    &messages[..historical_end],
                    &tool_definitions,
                    checkpoint.as_ref(),
                )?)
        {
            // The selected previous-model strategy may leave the new model's
            // auto-compaction threshold reached. Keep the exact normalized
            // fresh suffix separate when rechecking its installed window.
            let fresh = selected
                .as_ref()
                .map(|_| {
                    resource_context::for_compaction(
                        record,
                        checkpoint.as_ref(),
                        &messages[..historical_end],
                        &messages[historical_end..],
                        true,
                    )
                })
                .transpose()?;
            messages = self
                .compact_history_selected(
                    service,
                    record,
                    history,
                    checkpoint.as_ref(),
                    research_agent_core::context::compaction_strategy::CompactionPhase::PreTurn,
                    signal,
                    &messages[..historical_end],
                    &messages[historical_end..],
                    &tool_definitions,
                    retained_images_out,
                    selected.as_ref(),
                    selected.is_some(),
                )
                .await?;
            checkpoint = service
                .active_context_checkpoint(record.conversation_id)
                .await
                .map_err(|_| RunError::EventSink)?;
            if let Some(fresh) = fresh {
                let end = messages
                    .len()
                    .checked_sub(fresh.len())
                    .filter(|_| messages.ends_with(&fresh))
                    .ok_or(ModelError::InvalidRequest)?;
                if let Some(accounting) = self.transport.context_accounting() {
                    accounting
                        .prepare_auto_compaction(
                            &self.model,
                            &messages[..end],
                            &tool_definitions,
                            checkpoint.as_ref(),
                        )
                        .await?;
                }
                if self.should_compact_context(
                    &messages[..end],
                    &tool_definitions,
                    checkpoint.as_ref(),
                )? {
                    messages = self
                        .compact_history(
                            service,
                            record,
                            history,
                            checkpoint.as_ref(),
                            research_agent_core::context::compaction_strategy::CompactionPhase::PreTurn,
                            signal,
                            &messages[..end],
                            &messages[end..],
                            &tool_definitions,
                            retained_images_out,
                        )
                        .await?;
                    checkpoint = service
                        .active_context_checkpoint(record.conversation_id)
                        .await
                        .map_err(|_| RunError::EventSink)?;
                }
            }
        }
        // Like Codex, drain queued SessionStart hooks after both possible
        // pre-turn compactions and place their context before the new input.
        let hook_context = self
            .run_compact_session_start(service, record, signal)
            .await?;
        let position = messages
            .len()
            .checked_sub(1)
            .ok_or(ModelError::InvalidRequest)?;
        if !matches!(messages[position], ModelMessage::User { .. }) {
            return Err(ModelError::InvalidRequest.into());
        }
        messages.splice(position..position, hook_context);
        self.check_context_window(&messages, &tool_definitions, checkpoint.as_ref())?;
        if signal.is_cancelled() {
            return Err(ModelError::Canceled.into());
        }
        let request = ModelRequest {
            captured_history: Some(super::request_history_ref::capture(
                history,
                checkpoint
                    .as_ref()
                    .map_or(0, |checkpoint| checkpoint.source_watermark),
            )?),
            request_id: Uuid::new_v4(),
            model: self.model.clone(),
            messages,
            tools: tool_definitions,
            max_output_tokens: self.max_output_tokens,
            parallel_tool_calls: false,
            reasoning_effort: record.input.reasoning_effort.clone(),
        };
        request.validate(self.transport.capabilities())?;
        Ok(request)
    }
}

pub(super) fn project_history_tool_results(
    messages: &mut [ModelMessage],
    history: &ConversationProjection,
    policy: Option<research_agent_core::model::tool_output::ToolOutputPolicy>,
    hard_cap: usize,
) -> Result<(), RunError> {
    use research_agent_core::model::{history_tool_call_id, tool_output};
    use research_agent_core::protocol::ItemKind;
    let Some(policy) = policy else {
        return if tool_output::bound_tool_results(messages, hard_cap) {
            Ok(())
        } else {
            Err(RunError::Budget)
        };
    };
    let mut facts = HashMap::new();
    for turn_id in history.turn_order() {
        let Some(turn) = history.turn(*turn_id) else {
            continue;
        };
        let mut calls = HashMap::new();
        for (item_id, item) in &turn.items {
            if let ItemKind::ToolCall { call_id, .. } = &item.kind
                && item.completed
                && calls
                    .insert(*call_id, history_tool_call_id(*item_id))
                    .is_some()
            {
                return Err(ModelError::InvalidStream.into());
            }
        }
        for item in turn.items.values() {
            if let ItemKind::ToolResult {
                call_id,
                is_error: Some(error),
            } = &item.kind
                && item.completed
                && !turn.pending_tool_receipts.contains(call_id)
                && let Some(id) = calls.get(call_id)
                && facts
                    .insert(id.clone(), (item.text.as_str(), *error))
                    .is_some()
            {
                return Err(ModelError::InvalidStream.into());
            }
        }
    }
    for message in messages {
        if let ModelMessage::ToolResult {
            call_id,
            content,
            is_error,
        } = message
        {
            let source = facts.get(call_id.as_str()).filter(|(full, error)| {
                error == is_error && tool_output::is_bounded_tool_output(full, content)
            });
            let view = match source {
                Some((full, _)) => tool_output::project_tool_output(full, Some(policy), hard_cap),
                None => tool_output::project_tool_output(content, None, hard_cap),
            }
            .ok_or(RunError::Budget)?;
            *content = view;
        }
    }
    Ok(())
}
