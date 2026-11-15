pub(super) use super::terminal::latest_binding;
#[cfg(target_os = "macos")]
pub(super) use super::terminal::latest_for_scope;
use super::*;
use research_agent_core::protocol::{AgentEvent, EventPayload, ItemKind, TurnOutcome};

pub(super) fn storage(_: rusqlite::Error) -> Error {
    Error::Unavailable
}
pub(super) fn id(value: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| !id.is_nil() && id.to_string() == value)
        .ok_or(Error::Conflict)
}
fn event(raw: &str, scope: &Scope) -> Result<AgentEvent, Error> {
    let json: serde_json::Value = serde_json::from_str(raw).map_err(|_| Error::Conflict)?;
    if json["requestId"].as_str() != Some(scope.request_id.to_string().as_str()) {
        return Err(Error::Conflict);
    }
    let event: AgentEvent = serde_json::from_value(json).map_err(|_| Error::Conflict)?;
    if *event.conversation_id().as_uuid() != scope.conversation_id {
        return Err(Error::Conflict);
    }
    Ok(event)
}
pub(super) fn list(db: &Connection, request: Request) -> Result<Page, Error> {
    let mut query = db.prepare(
        "select d.id,w.conversation_id,w.request_id,w.context_snapshot_id,w.turn_id
         from writing_agent_file_deliveries d
         join agent_execution_workspaces w on w.conversation_id=d.conversation_id and w.request_id=d.request_id and w.manuscript_id=d.manuscript_id
         join agent_submissions s on s.conversation_id=w.conversation_id and s.request_id=w.request_id and s.turn_id=w.turn_id and s.context_snapshot_id=w.context_snapshot_id
         join manuscripts m on m.id=d.manuscript_id and m.status='active'
         join agent_conversations c on c.id=w.conversation_id and c.archived=0
         where d.manuscript_id=?1 and d.settled=0 and d.id>coalesce(?2,0)
         and exists(select 1 from agent_conversation_bindings b where b.conversation_id=c.id and b.resource_kind='manuscript' and b.resource_id=m.id)
         and exists(select 1 from agent_items i where i.conversation_id=w.conversation_id and json_extract(i.payload_json,'$.payload.type')='turnStarted' and json_extract(i.payload_json,'$.payload.turnId')=w.turn_id and json_extract(i.payload_json,'$.payload.contextSnapshotId')=w.context_snapshot_id and json_extract(i.payload_json,'$.requestId')=w.request_id)
         order by d.id limit ?3"
    ).map_err(storage)?;
    let rows = query
        .query_map(
            params![
                request.manuscript_id.to_string(),
                request.after,
                u16::from(request.limit) + 1
            ],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .map_err(storage)?;
    let mut items = Vec::new();
    for row in rows {
        let (ordinal, conversation, request_id, context, turn) = row.map_err(storage)?;
        if ordinal == 0 || ordinal > 9_007_199_254_740_991 {
            return Err(Error::Conflict);
        }
        let entry = Entry {
            id: ordinal,
            scope: Scope {
                manuscript_id: request.manuscript_id,
                conversation_id: id(&conversation)?,
                request_id: id(&request_id)?,
                context_snapshot_id: id(&context)?,
            },
            turn_id: id(&turn)?,
        };
        admitted(
            db,
            &Acknowledge {
                scope: entry.scope.clone(),
                turn_id: entry.turn_id,
            },
        )?;
        items.push(entry);
    }
    let more = items.len() > usize::from(request.limit);
    items.truncate(usize::from(request.limit));
    let next_cursor = if more {
        items.last().map(|item| item.id)
    } else {
        None
    };
    Ok(Page {
        schema_version: 1,
        manuscript_id: request.manuscript_id,
        items,
        next_cursor,
    })
}
fn admitted(db: &Connection, request: &Acknowledge) -> Result<(), Error> {
    let scope = &request.scope;
    let valid: bool = db.query_row(
        "select exists(select 1 from writing_agent_file_deliveries d join agent_execution_workspaces w on w.conversation_id=d.conversation_id and w.request_id=d.request_id join agent_conversation_bindings b on b.conversation_id=w.conversation_id join manuscripts m on m.id=w.manuscript_id where d.conversation_id=?1 and d.request_id=?2 and d.manuscript_id=?3 and w.manuscript_id=?3 and w.context_snapshot_id=?4 and w.turn_id=?5 and b.resource_kind='manuscript' and b.resource_id=m.id and m.status='active')",
        params![scope.conversation_id.to_string(), scope.request_id.to_string(), scope.manuscript_id.to_string(), scope.context_snapshot_id.to_string(), request.turn_id.to_string()], |row| row.get(0)
    ).map_err(storage)?;
    if !valid {
        return Err(Error::Denied);
    }
    let raw: String = db.query_row(
        "select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='turnStarted' and json_extract(payload_json,'$.payload.turnId')=?2",
        params![scope.conversation_id.to_string(), request.turn_id.to_string()], |row| row.get(0)
    ).optional().map_err(storage)?.ok_or(Error::Denied)?;
    let event = event(&raw, scope)?;
    if !matches!(event.payload(), EventPayload::TurnStarted { turn_id, context_snapshot_id, .. } if *turn_id==request.turn_id && *context_snapshot_id==scope.context_snapshot_id)
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

#[path = "repository/running.rs"]
mod running;
pub(super) use running::running_proof_with;

/// Resolve the executor's provider call ID to the one already accepted item.
/// The proof insert and this association share the caller's transaction.
pub(super) fn bind_running_call(
    db: &Connection,
    request: &Acknowledge,
    delivery_id: Uuid,
    provider_call_id: &str,
) -> Result<(), Error> {
    let scope = &request.scope;
    let mut statement = db.prepare("select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.requestId')=?2 and json_extract(payload_json,'$.payload.turnId')=?3 and json_extract(payload_json,'$.payload.type')='itemAccepted' and json_extract(payload_json,'$.payload.kind.type')='toolCall' and json_extract(payload_json,'$.payload.kind.providerCallId')=?4 order by ordinal limit 2").map_err(storage)?;
    let rows = statement
        .query_map(
            params![
                scope.conversation_id.to_string(),
                scope.request_id.to_string(),
                request.turn_id.to_string(),
                provider_call_id
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let [raw] = rows.as_slice() else {
        return Err(Error::Conflict);
    };
    let accepted = event(raw, scope)?;
    let EventPayload::ItemAccepted {
        turn_id,
        item_id,
        kind:
            ItemKind::ToolCall {
                call_id,
                name,
                provider_call_id: Some(original),
                namespace: None,
            },
        ..
    } = accepted.payload()
    else {
        return Err(Error::Conflict);
    };
    if *turn_id != request.turn_id
        || name != super::super::writing_delivery_tool::NAME
        || original != provider_call_id
        || item_id.is_nil()
        || call_id.is_nil()
    {
        return Err(Error::Conflict);
    }
    let inserted = db.execute("insert into writing_agent_delivery_tool_calls(delivery_id,item_id,call_id) values(?1,?2,?3) on conflict do nothing", params![delivery_id.to_string(), item_id.to_string(), call_id.to_string()]).map_err(storage)?;
    if inserted == 0 {
        let matches: bool = db.query_row("select exists(select 1 from writing_agent_delivery_tool_calls where delivery_id=?1 and item_id=?2 and call_id=?3)", params![delivery_id.to_string(), item_id.to_string(), call_id.to_string()], |row| row.get(0)).map_err(storage)?;
        if !matches {
            return Err(Error::Conflict);
        }
    }
    Ok(())
}

/// Persist only the already-completed result of the proof's exact accepted call.
/// The shared projection enforces journal ordering, pairing and receipt state.
pub(super) fn confirm_result(
    db: &Connection,
    record: &research_agent_core::submission::SubmissionRecord,
    provider_call_id: &str,
) -> Result<(), Error> {
    use research_agent_core::{conversation::ConversationId, protocol::ConversationProjection};
    if provider_call_id.is_empty() {
        return Err(Error::InvalidInput);
    }
    let conversation = ConversationId::from_uuid(record.conversation_id);
    let mut projection = ConversationProjection::new(conversation);
    let mut statement = db.prepare("select ordinal,schema_version,payload_json from agent_items where conversation_id=?1 order by ordinal").map_err(storage)?;
    let rows = statement
        .query_map([record.conversation_id.to_string()], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(storage)?;
    for row in rows {
        let (ordinal, version, json) = row.map_err(storage)?;
        let event: AgentEvent = serde_json::from_str(&json).map_err(|_| Error::Conflict)?;
        if version != 1 || ordinal != projection.seq() || event.seq() != ordinal + 1 {
            return Err(Error::Conflict);
        }
        projection.apply(&event).map_err(|_| Error::Conflict)?;
    }
    if projection.active_turn() != Some(record.turn_id) {
        return Err(Error::Denied);
    }
    let turn = projection.turn(record.turn_id).ok_or(Error::Conflict)?;
    if turn.context_snapshot_id != record.context_snapshot_id {
        return Err(Error::Conflict);
    }
    let mut calls = turn
        .items
        .iter()
        .filter_map(|(item_id, item)| match &item.kind {
            ItemKind::ToolCall {
                call_id,
                name,
                provider_call_id: Some(original),
                namespace: None,
            } if item.accepted
                && name == super::super::writing_delivery_tool::NAME
                && original == provider_call_id =>
            {
                Some((*item_id, *call_id))
            }
            _ => None,
        });
    let (item_id, call_id) = calls.next().ok_or(Error::Conflict)?;
    if calls.next().is_some() || !turn.pending_tool_receipts.contains(&call_id) {
        return Err(Error::Conflict);
    }
    let saved: Option<(String, String)> = db.query_row("select p.delivery_id,p.proof_json from writing_agent_delivery_proofs p join writing_agent_delivery_tool_calls c on c.delivery_id=p.delivery_id where p.conversation_id=?1 and p.request_id=?2 and p.turn_id=?3 and p.context_snapshot_id=?4 and c.item_id=?5 and c.call_id=?6", params![record.conversation_id.to_string(), record.request_id.to_string(), record.turn_id.to_string(), record.context_snapshot_id.to_string(), item_id.to_string(), call_id.to_string()], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(storage)?;
    let (delivery, proof) = saved.ok_or(Error::Conflict)?;
    let delivery = id(&delivery)?;
    let proof: serde_json::Value = serde_json::from_str(&proof).map_err(|_| Error::Conflict)?;
    let status = match (proof["schemaVersion"].as_u64(), proof["kind"].as_str()) {
        (Some(1), Some("applied")) => "complete",
        (Some(1), Some("rejected")) => "rejected",
        (Some(1), Some("empty")) => "unchanged",
        _ => return Err(Error::Conflict),
    };
    let mut results = turn.items.iter().filter(
        |(_, item)| matches!(item.kind, ItemKind::ToolResult { call_id: id, .. } if id == call_id),
    );
    let (result_id, result) = results.next().ok_or(Error::Conflict)?;
    if results.next().is_some()
        || !result.completed
        || !matches!(
            result.kind,
            ItemKind::ToolResult {
                is_error: Some(false),
                ..
            }
        )
    {
        return Err(Error::Conflict);
    }
    let wrong_request: bool = db.query_row(
        "select exists(select 1 from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 and (json_extract(payload_json,'$.payload.itemId') in (?3,?4) or (json_extract(payload_json,'$.payload.type')='toolReceiptPending' and json_extract(payload_json,'$.payload.callId')=?5)) and coalesce(json_extract(payload_json,'$.requestId'),'') != ?6)",
        params![record.conversation_id.to_string(),record.turn_id.to_string(),item_id.to_string(),result_id.to_string(),call_id.to_string(),record.request_id.to_string()], |row| row.get(0)).map_err(storage)?;
    if wrong_request {
        return Err(Error::Conflict);
    }
    let body: serde_json::Value =
        serde_json::from_str(&result.text).map_err(|_| Error::Conflict)?;
    if body["status"].as_str() != Some(status)
        || body["requestId"].as_str() != Some(record.request_id.to_string().as_str())
    {
        return Err(Error::Conflict);
    }
    let inserted = db.execute("insert into writing_agent_delivery_results(delivery_id,result_item_id) values(?1,?2) on conflict do nothing", params![delivery.to_string(), result_id.to_string()]).map_err(storage)?;
    if inserted == 0 {
        let matches: bool = db.query_row("select exists(select 1 from writing_agent_delivery_results where delivery_id=?1 and result_item_id=?2)", params![delivery.to_string(), result_id.to_string()], |row| row.get(0)).map_err(storage)?;
        if !matches {
            return Err(Error::Conflict);
        }
    }
    Ok(())
}
/// False means completed with no proposal: caller must independently verify an empty native copy.
pub(super) fn settlement_proven(db: &Connection, request: &Acknowledge) -> Result<bool, Error> {
    admitted(db, request)?;
    let scope = &request.scope;
    let raw: String = db.query_row(
        "select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='turnFinished' and json_extract(payload_json,'$.payload.turnId')=?2",
        params![scope.conversation_id.to_string(), request.turn_id.to_string()], |row| row.get(0)
    ).optional().map_err(storage)?.ok_or(Error::Denied)?;
    let event = event(&raw, scope)?;
    match event.payload() {
        EventPayload::TurnFinished { turn_id, outcome } if *turn_id == request.turn_id => {
            if *outcome != TurnOutcome::Completed {
                return Ok(true);
            }
        }
        _ => return Err(Error::Conflict),
    }
    let binding = latest_binding(db, request)?;
    let proposal: Option<String> = db.query_row(
        "select id from agent_change_sets where conversation_id=?1 and request_id=?2 and manuscript_id=?3 and context_snapshot_id=?4 and turn_id=?5 and lease_id=?6 and manifest_hash=?7 order by created_at desc,id desc limit 1",
        params![scope.conversation_id.to_string(), scope.request_id.to_string(), scope.manuscript_id.to_string(), scope.context_snapshot_id.to_string(), request.turn_id.to_string(), binding.lease_id.to_string(), binding.manifest_hash], |row| row.get(0)
    ).optional().map_err(storage)?;
    let Some(proposal) = proposal else {
        return Ok(false);
    };
    let proposal = id(&proposal)?;
    use super::super::repository::{change_sets, proposal_applications};
    let frozen = change_sets::read_in(db, scope.conversation_id, scope.manuscript_id, proposal)
        .map_err(|_| Error::Conflict)?;
    if matches!(
        frozen.status,
        change_sets::ChangeSetStatus::Rejected | change_sets::ChangeSetStatus::Undone
    ) {
        return Ok(true);
    }
    let application =
        proposal_applications::find_in(db, scope.conversation_id, scope.manuscript_id, proposal)
            .map_err(|_| Error::Conflict)?;
    if application
        .is_some_and(|value| value.status == proposal_applications::ApplicationStatus::Applied)
    {
        Ok(true)
    } else {
        Err(Error::RecoveryRequired)
    }
}

pub(super) fn acknowledge(
    db: &Connection,
    request: &Acknowledge,
    empty_copy_verified: bool,
) -> Result<(), Error> {
    if !settlement_proven(db, request)? && !empty_copy_verified {
        return Err(Error::Denied);
    }
    db.execute("update writing_agent_file_deliveries set settled=1 where conversation_id=?1 and request_id=?2 and manuscript_id=?3",
        params![request.scope.conversation_id.to_string(), request.scope.request_id.to_string(), request.scope.manuscript_id.to_string()]).map_err(storage)?;
    Ok(())
}
