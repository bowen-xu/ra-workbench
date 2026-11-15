//! Restore confirmed predecessor state from authoritative journal order, never clocks.
use super::*;
use research_agent_core::{
    conversation::ConversationId,
    model::history_tool_call_id,
    protocol::{AgentEvent, ConversationProjection, EventPayload, ItemKind, TurnOutcome},
};
use std::collections::HashMap;

pub(super) fn initial_state(
    db: &Connection,
    scope: &ResponsesStateScope,
    max_bytes: usize,
    active_window: Option<&str>,
) -> Result<ResponsesContinuation, ModelError> {
    let history = prior_history(db, scope, max_bytes)?;
    for previous_id in history.turn_order().iter().rev() {
        let previous = history
            .turn(*previous_id)
            .ok_or(ModelError::InvalidStream)?;
        let inherited: bool = db
            .query_row(
                "select exists(
                    select 1 from agent_fork_turn_sources f
                    join agent_submissions s on s.conversation_id=f.source_conversation_id
                        and s.turn_id=f.turn_id and s.context_snapshot_id=f.context_snapshot_id
                    where f.conversation_id=?1 and f.turn_id=?2 and f.context_snapshot_id=?3
                        and f.source_conversation_id<>f.conversation_id and s.schema_version=1
                        and not exists(select 1 from agent_submissions owned
                            where owned.conversation_id=f.conversation_id and owned.turn_id=f.turn_id)
                )",
                params![scope.conversation_id.to_string(), previous_id.to_string(), previous.context_snapshot_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| ModelError::Unavailable)?;
        if inherited {
            // Forks retain frozen history, never the parent's runtime receipts.
            // Reconstruct that history from normalized input or the child-owned
            // replacement window; only locally executed turns can restore wire state.
            continue;
        }
        if previous.model != scope.model {
            // A model switch uses normalized history; opaque state stays model scoped.
            return ResponsesContinuation::new(scope.turn_id, scope.model.clone(), max_bytes);
        }
        let bound_window: Option<String> = db
            .query_row(
                "select window_id from agent_responses_window_bindings where turn_id=?1",
                [previous_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| ModelError::Unavailable)?;
        if bound_window.as_deref() != active_window {
            // Replacement checkpoint is authoritative. Never inherit an old wire
            // prefix simply because its model matches the new normalized request.
            continue;
        }
        let saved: Option<(u32, String)> = db
            .query_row(
                "select p.schema_version,p.checkpoint_json from agent_responses_checkpoints p
         join agent_submissions s on s.turn_id=p.turn_id
         where s.conversation_id=?1 and s.turn_id=?2 and s.context_snapshot_id=?3",
                params![
                    scope.conversation_id.to_string(),
                    previous_id.to_string(),
                    previous.context_snapshot_id.to_string()
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| ModelError::Unavailable)?;
        if previous.outcome != Some(TurnOutcome::Completed) {
            // A failed admission/headers wait, or an interrupted turn with a
            // provider checkpoint, cannot safely continue from opaque provider
            // state. Preserve only the normalized history projection and seek the
            // earlier confirmed state; this never replays the interrupted request.
            if previous.outcome.is_none()
                || previous.items.values().any(|item| {
                    !item.completed
                        && !matches!(
                            item.kind,
                            ItemKind::AssistantMessage | ItemKind::ContextCompaction
                        )
                })
            {
                return Err(ModelError::InvalidRequest);
            }
            continue;
        }
        let state = match saved {
            Some((1..=3, encoded)) => Some(ResponsesContinuation::restore(
                *previous_id,
                scope.model.clone(),
                max_bytes,
                &encoded,
            )?),
            Some(_) => return Err(ModelError::InvalidStream),
            None => None,
        };
        let saved_id = state
            .as_ref()
            .map(ResponsesContinuation::saved_request_id)
            .transpose()?
            .flatten();
        let state = state.ok_or(ModelError::InvalidRequest)?;
        let saved_id = saved_id.ok_or(ModelError::InvalidRequest)?;
        let position = previous
            .attempt_order
            .iter()
            .position(|id| *id == saved_id)
            .ok_or(ModelError::InvalidStream)?;
        if previous
            .attempts
            .get(&saved_id)
            .is_none_or(|attempt| attempt.outcome != Some(TurnOutcome::Completed))
            || previous.attempt_order[position + 1..].iter().any(|id| {
                previous
                    .attempts
                    .get(id)
                    .is_none_or(|attempt| attempt.response_id.is_some())
            })
        {
            return Err(ModelError::InvalidRequest);
        }
        // The journal remains complete, but covered calls/results no longer belong
        // to the installed provider prefix. Replaying them after mid-turn compaction
        // would either fail call identity validation or reintroduce old tool output.
        let replay_history = match active_window {
            Some(window_id) => {
                let checkpoint =
                    super::super::context_windows::read_active(db, scope.conversation_id)
                        .map_err(|_| ModelError::InvalidStream)?
                        .ok_or(ModelError::InvalidStream)?;
                if checkpoint.window_id.to_string() != window_id {
                    return Err(ModelError::InvalidStream);
                }
                history
                    .context_tail(checkpoint.source_watermark)
                    .map_err(|_| ModelError::InvalidStream)?
            }
            None => history.clone(),
        };
        let mut aliases = HashMap::new();
        let mut original_calls = HashMap::new();
        let replay = replay_history.turn(*previous_id);
        for (item_id, item) in replay.into_iter().flat_map(|turn| turn.items.iter()) {
            if let ItemKind::ToolCall {
                provider_call_id,
                call_id,
                ..
            } = &item.kind
            {
                let original = provider_call_id.as_ref().ok_or(ModelError::InvalidStream)?;
                if !item.completed
                    || aliases
                        .insert(original.clone(), history_tool_call_id(*item_id))
                        .is_some()
                {
                    return Err(ModelError::InvalidStream);
                }
                original_calls.insert(*call_id, original.clone());
            }
        }
        let mut results = Vec::new();
        for item_id in replay.into_iter().flat_map(|turn| turn.item_order.iter()) {
            let item = replay
                .ok_or(ModelError::InvalidStream)?
                .items
                .get(item_id)
                .ok_or(ModelError::InvalidStream)?;
            if let ItemKind::ToolResult { call_id, is_error } = item.kind {
                if !item.completed {
                    return Err(ModelError::InvalidRequest);
                }
                let id = original_calls
                    .get(&call_id)
                    .ok_or(ModelError::InvalidStream)?;
                let output = match is_error {
                    Some(false) => item.text.clone(),
                    Some(true) => {
                        serde_json::json!({"is_error":true,"content":item.text}).to_string()
                    }
                    None => return Err(ModelError::InvalidRequest),
                };
                results.push(
                    serde_json::json!({"type":"function_call_output","call_id":id,"output":output}),
                );
            }
        }
        return state
            .with_durable_tool_results(&results)?
            .inherit_for_turn(scope.turn_id, &aliases);
    }
    ResponsesContinuation::new(scope.turn_id, scope.model.clone(), max_bytes)
}

fn prior_history(
    db: &Connection,
    scope: &ResponsesStateScope,
    max_bytes: usize,
) -> Result<ConversationProjection, ModelError> {
    let mut projection =
        ConversationProjection::new(ConversationId::from_uuid(scope.conversation_id));
    let mut statement = db.prepare(
        "select ordinal,schema_version,payload_json from agent_items where conversation_id=?1 order by ordinal"
    ).map_err(|_| ModelError::Unavailable)?;
    let rows = statement
        .query_map([scope.conversation_id.to_string()], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| ModelError::Unavailable)?;
    let mut bytes = 0usize;
    for row in rows {
        let (ordinal, version, encoded) = row.map_err(|_| ModelError::Unavailable)?;
        bytes = bytes
            .checked_add(encoded.len())
            .ok_or(ModelError::BudgetExceeded)?;
        if bytes > max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        let event: AgentEvent =
            serde_json::from_str(&encoded).map_err(|_| ModelError::InvalidStream)?;
        if version != 1 || ordinal != projection.seq() || event.seq() != ordinal + 1 {
            return Err(ModelError::InvalidStream);
        }
        if let EventPayload::TurnStarted {
            turn_id,
            context_snapshot_id,
            model,
            ..
        } = event.payload()
            && *turn_id == scope.turn_id
        {
            if projection.active_turn().is_some()
                || *event.conversation_id().as_uuid() != scope.conversation_id
                || *context_snapshot_id != scope.context_snapshot_id
                || *model != scope.model
            {
                return Err(ModelError::InvalidStream);
            }
            return Ok(projection);
        }
        projection
            .apply(&event)
            .map_err(|_| ModelError::InvalidStream)?;
    }
    Err(ModelError::InvalidRequest)
}
