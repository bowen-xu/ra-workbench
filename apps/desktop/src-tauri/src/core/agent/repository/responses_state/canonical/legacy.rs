//! Old opaque material is selected through frozen turn/item handles. Prefix
//! equality validates that selected record; it never discovers record identity.
use super::*;
use crate::agent::model::responses_metadata::CapturedResponsesInput;
use research_agent_core::model::history_tool_call_id;
use std::collections::{HashMap, HashSet};
#[path = "legacy/tool_results.rs"]
mod tool_results;

struct Fact {
    seq: u64,
    kind: String,
    attempt: Option<String>,
    item: Option<Uuid>,
    provider_call: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    db: &Connection,
    scope: &ResponsesStateScope,
    max_bytes: usize,
    account: Uuid,
    input: &[Value],
    captured: &CapturedResponsesInput,
    active_window: Option<&str>,
    policy: Option<crate::agent::execution_profile::NativeCompactionPolicy>,
) -> Result<Option<Vec<Value>>, ModelError> {
    let Some(history) = &captured.history else {
        let exists:bool=db.query_row("select exists(select 1 from agent_responses_checkpoints p join agent_submissions s on s.turn_id=p.turn_id where s.conversation_id=?1 and json_type(p.checkpoint_json,'$.checkpoint')='object')",[scope.conversation_id.to_string()],|row|row.get(0)).map_err(|_|ModelError::Unavailable)?;
        return if exists {
            Err(ModelError::UnsupportedCapability)
        } else {
            Ok(None)
        };
    };
    if history.conversation_id != scope.conversation_id || history.after_seq > history.through_seq {
        return Err(ModelError::InvalidRequest);
    }
    let durable: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [scope.conversation_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| ModelError::Unavailable)?;
    if history.through_seq > durable {
        return Err(ModelError::InvalidRequest);
    }
    let mut seen = HashSet::new();
    let mut unmatched = false;
    for turn in history.turns.iter().rev() {
        if turn.turn_id.is_nil() || !seen.insert(turn.turn_id) {
            return Err(ModelError::InvalidRequest);
        }
        let source:Option<String>=db.query_row("select source_conversation_id from agent_fork_turn_sources where conversation_id=?1 and turn_id=?2",params![scope.conversation_id.to_string(),turn.turn_id.to_string()],|row|row.get(0)).optional().map_err(|_|ModelError::Unavailable)?;
        let source = source.unwrap_or_else(|| scope.conversation_id.to_string());
        let saved:Option<(u32,String,Option<String>)>=db.query_row("select p.schema_version,p.checkpoint_json,b.window_id from agent_responses_checkpoints p join agent_submissions s on s.turn_id=p.turn_id left join agent_responses_window_bindings b on b.turn_id=p.turn_id where s.conversation_id=?1 and p.turn_id=?2",params![source,turn.turn_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|ModelError::Unavailable)?;
        let Some((version, encoded, window)) = saved else {
            continue;
        };
        if source == scope.conversation_id.to_string() && window.as_deref() != active_window {
            continue;
        }
        if !matches!(version, 1..=3) {
            return Err(ModelError::UnsupportedCapability);
        }
        if encoded.len() > max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        let raw: Value = serde_json::from_str(&encoded).map_err(|_| ModelError::InvalidStream)?;
        if raw["checkpoint"].is_null() {
            continue;
        }
        let request = raw["checkpoint"]["request_id"]
            .as_str()
            .ok_or(ModelError::InvalidStream)?;
        let source_seq:Option<u64>=db.query_row("select ordinal+1 from agent_items where conversation_id=?1 and ordinal<?2 and json_extract(payload_json,'$.payload.type')='modelRequestStarted' and json_extract(payload_json,'$.payload.turnId')=?3 and json_extract(payload_json,'$.payload.attemptId')=?4 order by ordinal limit 1",
            params![scope.conversation_id.to_string(),i64::try_from(history.through_seq).map_err(|_|ModelError::InvalidRequest)?,turn.turn_id.to_string(),request],|row|row.get(0)).optional().map_err(|_|ModelError::Unavailable)?;
        if source_seq.is_some_and(|seq| seq <= history.after_seq) {
            continue;
        }
        let facts = facts(
            db,
            scope,
            turn.turn_id,
            history.after_seq,
            history.through_seq,
        )?;
        let Some(start) = facts
            .iter()
            .find(|fact| {
                fact.kind == "modelRequestStarted" && fact.attempt.as_deref() == Some(request)
            })
            .map(|fact| fact.seq)
        else {
            // An inherited admission without a response may name an earlier
            // request. Try its separately captured source turn, never latest SQL.
            unmatched = true;
            continue;
        };
        let boundary = facts
            .iter()
            .find(|fact| {
                fact.seq > start
                    && ((matches!(
                        fact.kind.as_str(),
                        "modelRequestFinished" | "modelRequestFailed"
                    ) && fact.attempt.as_deref() == Some(request))
                        || fact.kind == "modelRequestStarted"
                        || fact.kind == "turnFinished")
            })
            .map_or(history.through_seq, |fact| fact.seq);
        let admitted = facts
            .iter()
            .filter(|fact| fact.seq <= boundary)
            .filter_map(|fact| fact.item)
            .collect::<HashSet<_>>();
        let mut covered_end = None;
        for item in &captured.items {
            if admitted.contains(&item.item_id) {
                covered_end = Some(covered_end.map_or(item.input_index + 1, |end: usize| {
                    end.max(item.input_index + 1)
                }));
            }
        }
        for user in &captured.users {
            if turn.user_item_ids.contains(&user.item_id) && admitted.contains(&user.item_id) {
                covered_end = Some(
                    covered_end.map_or(user.input_index + 1, |end| end.max(user.input_index + 1)),
                );
            }
        }
        let covered_end = covered_end.ok_or(ModelError::UnsupportedCapability)?;
        if captured
            .items
            .iter()
            .any(|item| item.native && item.input_index < covered_end)
        {
            // Never let an older prefix overwrite a newer canonical fact.
            return Err(ModelError::UnsupportedCapability);
        }
        let stored_model: ModelIdentity =
            serde_json::from_value(raw["model"].clone()).map_err(|_| ModelError::InvalidStream)?;
        if stored_model != scope.model {
            return Err(ModelError::UnsupportedCapability);
        }
        let state =
            ResponsesContinuation::restore(turn.turn_id, scope.model.clone(), max_bytes, &encoded)?;
        compaction::bind_installed(&state, db, scope, account, policy)?;
        let aliases = aliases(
            db,
            scope,
            captured,
            history.after_seq,
            history.through_seq,
            input,
        )?;
        let direct = state.prepare_captured_legacy(
            scope.turn_id,
            account,
            scope.model.clone(),
            crate::agent::model::responses_continuation::CapturedLegacyInput {
                input: input.to_vec(),
                authority_items: captured.authority_items,
                covered_end,
            },
            &aliases,
            &HashMap::new(),
        );
        match direct {
            Ok(result) => return Ok(Some(result)),
            Err(ModelError::InvalidStream | ModelError::InvalidRequest) => {}
            Err(error) => return Err(error),
        }
        // Only a mismatching legacy prefix needs the bounded, identity-scoped
        // durable result lookup. Normal requests never rescan tool bodies.
        let results =
            tool_results::read(db, scope, captured, history.after_seq, history.through_seq)?;
        let state =
            ResponsesContinuation::restore(turn.turn_id, scope.model.clone(), max_bytes, &encoded)?;
        compaction::bind_installed(&state, db, scope, account, policy)?;
        return state
            .prepare_captured_legacy(
                scope.turn_id,
                account,
                scope.model.clone(),
                crate::agent::model::responses_continuation::CapturedLegacyInput {
                    input: input.to_vec(),
                    authority_items: captured.authority_items,
                    covered_end,
                },
                &aliases,
                &results,
            )
            .map(Some);
    }
    if unmatched {
        Err(ModelError::UnsupportedCapability)
    } else {
        Ok(None)
    }
}

fn facts(
    db: &Connection,
    scope: &ResponsesStateScope,
    turn: Uuid,
    after: u64,
    through: u64,
) -> Result<Vec<Fact>, ModelError> {
    let mut query=db.prepare("select ordinal+1,json_extract(payload_json,'$.payload.type'),json_extract(payload_json,'$.payload.attemptId'),json_extract(payload_json,'$.payload.itemId'),json_extract(payload_json,'$.payload.kind.providerCallId') from agent_items where conversation_id=?1 and ordinal>=?2 and ordinal<?3 and json_extract(payload_json,'$.payload.turnId')=?4 and json_extract(payload_json,'$.payload.type') in ('modelRequestStarted','modelRequestFinished','modelRequestFailed','turnFinished','itemStarted','itemAccepted') order by ordinal").map_err(|_|ModelError::Unavailable)?;
    query
        .query_map(
            params![
                scope.conversation_id.to_string(),
                i64::try_from(after).map_err(|_| ModelError::InvalidRequest)?,
                i64::try_from(through).map_err(|_| ModelError::InvalidRequest)?,
                turn.to_string()
            ],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .map_err(|_| ModelError::Unavailable)?
        .map(|row| {
            let (seq, kind, attempt, item, provider_call) =
                row.map_err(|_| ModelError::Unavailable)?;
            Ok(Fact {
                seq,
                kind,
                attempt,
                item: item
                    .map(|id| Uuid::parse_str(&id).map_err(|_| ModelError::InvalidStream))
                    .transpose()?,
                provider_call,
            })
        })
        .collect()
}

fn aliases(
    db: &Connection,
    scope: &ResponsesStateScope,
    captured: &CapturedResponsesInput,
    after: u64,
    through: u64,
    input: &[Value],
) -> Result<HashMap<String, String>, ModelError> {
    let history = captured
        .history
        .as_ref()
        .ok_or(ModelError::InvalidRequest)?;
    let ids = captured
        .items
        .iter()
        .map(|item| item.item_id)
        .collect::<HashSet<_>>();
    let mut aliases = HashMap::new();
    // A replacement window can retain stable call IDs without a chat-item row.
    // These are frozen protocol identities, never a content-based lookup.
    for item in input {
        if item["type"] == "function_call"
            && let Some(id) = item["call_id"].as_str()
        {
            aliases.insert(id.to_owned(), id.to_owned());
        }
    }
    for turn in &history.turns {
        for fact in facts(db, scope, turn.turn_id, after, through)? {
            if let Some(item) = fact.item
                && ids.contains(&item)
            {
                let stable = history_tool_call_id(item);
                aliases.insert(stable.clone(), stable.clone());
                if let Some(id) = fact.provider_call {
                    aliases.insert(id, stable);
                }
            }
        }
    }
    Ok(aliases)
}
