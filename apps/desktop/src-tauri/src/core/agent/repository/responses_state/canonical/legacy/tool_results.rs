//! Read completed tool facts only through captured item identities and watermark.
use super::*;
use research_agent_core::protocol::{AgentEvent, EventPayload, ItemKind};

struct ResultDraft {
    pair: Uuid,
    error: Option<bool>,
    text: String,
    complete: bool,
}

pub(super) fn read(
    db: &Connection,
    scope: &ResponsesStateScope,
    captured: &CapturedResponsesInput,
    after: u64,
    through: u64,
) -> Result<HashMap<String, (String, bool)>, ModelError> {
    let history = captured
        .history
        .as_ref()
        .ok_or(ModelError::InvalidRequest)?;
    let selected: HashSet<_> = captured.items.iter().map(|item| item.item_id).collect();
    let mut output = HashMap::new();
    for turn in &history.turns {
        let mut calls = HashMap::new();
        let mut call_items = HashMap::new();
        let mut drafts: HashMap<Uuid, ResultDraft> = HashMap::new();
        let mut pending = HashSet::new();
        let mut query = db.prepare("select payload_json from agent_items where conversation_id=?1 and ordinal>=?2 and ordinal<?3 and json_extract(payload_json,'$.payload.turnId')=?4 and json_extract(payload_json,'$.payload.type') in ('itemStarted','itemAccepted','textDelta','itemCompleted','toolReceiptPending','toolReceiptConfirmed') order by ordinal").map_err(|_| ModelError::Unavailable)?;
        let rows = query
            .query_map(
                params![
                    scope.conversation_id.to_string(),
                    i64::try_from(after).map_err(|_| ModelError::InvalidRequest)?,
                    i64::try_from(through).map_err(|_| ModelError::InvalidRequest)?,
                    turn.turn_id.to_string()
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| ModelError::Unavailable)?;
        for row in rows {
            let event: AgentEvent =
                serde_json::from_str(&row.map_err(|_| ModelError::Unavailable)?)
                    .map_err(|_| ModelError::InvalidStream)?;
            match event.payload() {
                EventPayload::ItemStarted {
                    item_id,
                    kind: ItemKind::ToolCall { call_id, .. },
                    ..
                } if selected.contains(item_id) => {
                    match (calls.entry(*call_id), call_items.entry(*item_id)) {
                        (
                            std::collections::hash_map::Entry::Vacant(call),
                            std::collections::hash_map::Entry::Vacant(item),
                        ) => {
                            call.insert((history_tool_call_id(*item_id), false));
                            item.insert(*call_id);
                        }
                        _ => return Err(ModelError::InvalidStream),
                    };
                }
                EventPayload::ItemAccepted {
                    item_id,
                    kind: ItemKind::ToolCall { call_id, .. },
                    ..
                } if selected.contains(item_id) => {
                    let stable = history_tool_call_id(*item_id);
                    if let Some((existing, complete)) = calls.get_mut(call_id) {
                        if *existing != stable || *complete {
                            return Err(ModelError::InvalidStream);
                        }
                        *complete = true;
                    } else {
                        calls.insert(*call_id, (stable, true));
                    }
                    call_items.insert(*item_id, *call_id);
                }
                EventPayload::ItemStarted {
                    item_id,
                    kind: ItemKind::ToolResult { call_id, is_error },
                    ..
                } if calls.contains_key(call_id) => match drafts.entry(*item_id) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(ResultDraft {
                            pair: *call_id,
                            error: *is_error,
                            text: String::new(),
                            complete: false,
                        });
                    }
                    std::collections::hash_map::Entry::Occupied(_) => {
                        return Err(ModelError::InvalidStream);
                    }
                },
                EventPayload::TextDelta { item_id, text, .. } => {
                    if let Some(draft) = drafts.get_mut(item_id) {
                        if draft.complete
                            || draft.text.len().saturating_add(text.len())
                                > research_agent_core::protocol::MAX_NATIVE_ITEM_BYTES
                        {
                            return Err(ModelError::BudgetExceeded);
                        }
                        draft.text.push_str(text);
                    }
                }
                EventPayload::ItemCompleted { item_id, .. } => {
                    if let Some(draft) = drafts.get_mut(item_id) {
                        draft.complete = true;
                    }
                    if let Some(pair) = call_items.get(item_id)
                        && let Some((_, complete)) = calls.get_mut(pair)
                    {
                        *complete = true;
                    }
                }
                EventPayload::ToolReceiptPending { call_id, .. } => {
                    pending.insert(*call_id);
                }
                EventPayload::ToolReceiptConfirmed { call_id, .. } => {
                    pending.remove(call_id);
                }
                _ => {}
            }
        }
        for draft in drafts.into_values() {
            if !draft.complete || pending.contains(&draft.pair) {
                continue;
            }
            if let (Some((id, true)), Some(error)) = (calls.get(&draft.pair), draft.error)
                && output.insert(id.clone(), (draft.text, error)).is_some()
            {
                return Err(ModelError::InvalidStream);
            }
        }
    }
    Ok(output)
}
