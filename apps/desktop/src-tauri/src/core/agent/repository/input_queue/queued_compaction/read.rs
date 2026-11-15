use super::*;
use research_agent_core::protocol::{AgentEvent, EventPayload, TaskKind};

pub(super) fn preparation(
    db: &Connection,
    conversation: Uuid,
    input: Uuid,
    request: Uuid,
) -> Result<Option<(PrepareRecord, InputQueue)>, SqlMutationError> {
    let raw:Option<(String,String,String)>=db.query_row("select conversation_id,command_json,result_json from agent_input_queue_receipts where request_id=?1",[request.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((scope, command, json)) = raw else {
        return Ok(None);
    };
    let record: PrepareRecord =
        serde_json::from_str(&command).map_err(|_| CatalogError::Conflict)?;
    if scope != conversation.to_string()
        || record.kind != "queuedCompactionPrepare:v1"
        || record.conversation_id != conversation
        || record.input_id != input
        || record.request_id != request
    {
        return Err(CatalogError::Conflict.into());
    }
    let snapshot = Snapshot::decode(&json, conversation).map_err(domain)?;
    if snapshot.revision()
        != record
            .expected_queue_revision
            .checked_add(1)
            .ok_or(CatalogError::Corrupt)?
        || snapshot.preparing() != Some(input)
    {
        return Err(CatalogError::Corrupt.into());
    }
    Ok(Some((record, snapshot)))
}
pub(super) fn terminal(
    db: &Connection,
    record: &SubmissionRecord,
) -> Result<Option<TurnOutcome>, SqlMutationError> {
    let mut statement=db.prepare("select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 and json_extract(payload_json,'$.payload.type') in ('turnStarted','turnFinished') order by ordinal limit 3")?;
    let rows = statement.query_map(
        params![
            record.conversation_id.to_string(),
            record.turn_id.to_string()
        ],
        |row| row.get::<_, String>(0),
    )?;
    let events = rows
        .map(|row| {
            let event: AgentEvent =
                serde_json::from_str(&row?).map_err(|_| CatalogError::Corrupt)?;
            if event.conversation_id().as_uuid() != &record.conversation_id {
                return Err(CatalogError::Corrupt.into());
            }
            Ok(event)
        })
        .collect::<Result<Vec<_>, SqlMutationError>>()?;
    let valid_start = |event: &AgentEvent| {
        event.request_id() == record.request_id
            && matches!(event.payload(),EventPayload::TurnStarted{turn_id,task_kind:TaskKind::Compact,context_snapshot_id,model} if *turn_id==record.turn_id&&*context_snapshot_id==record.context_snapshot_id&&*model==record.input.model)
    };
    match events.as_slice() {
        [] => Ok(None),
        [start] if valid_start(start) => Ok(None),
        // Native recovery commits TurnFinished under its own operation request
        // ID. The original start binds C/R/turn/purpose; finish binds that turn.
        [start, finished] if valid_start(start) => match finished.payload() {
            EventPayload::TurnFinished { turn_id, outcome } if *turn_id == record.turn_id => {
                Ok(Some(*outcome))
            }
            _ => Err(CatalogError::Corrupt.into()),
        },
        _ => Err(CatalogError::Corrupt.into()),
    }
}
pub(crate) fn read_in(
    db: &Connection,
    intent: &ReadIntent,
) -> Result<Option<QueuedCompactionRead>, SqlMutationError> {
    types::ids(&[
        intent.conversation_id,
        intent.manuscript_id,
        intent.input_id,
    ])?;
    if intent.request_id.is_some_and(|id| id.is_nil()) {
        return Err(CatalogError::InvalidInput.into());
    }
    authorize(db, intent.conversation_id, intent.manuscript_id)?;
    let pointer = current_request(db, intent.conversation_id, intent.input_id)?;
    let latest = queue(db, intent.conversation_id)?;
    let request = match intent.request_id.or(pointer) {
        Some(request) => Some(request),
        None if latest
            .inputs()
            .iter()
            .any(|input| input.id == intent.input_id) =>
        {
            // Settlement removes the active pointer, but a retained I still
            // needs its latest original R for an explicit new attempt.
            let previous:Option<String>=db.query_row("select request_id from agent_queue_submission_origins where conversation_id=?1 and input_id=?2 order by rowid desc limit 1",params![intent.conversation_id.to_string(),intent.input_id.to_string()],|row|row.get(0)).optional()?;
            if previous.is_none() {
                reject_legacy_candidate(db, intent.conversation_id, intent.input_id)?;
            }
            previous
                .map(|value| Uuid::parse_str(&value).map_err(|_| CatalogError::Corrupt))
                .transpose()?
        }
        None => None,
    };
    let Some(request) = request else {
        return Ok(None);
    };
    let origin:Option<String>=db.query_row("select input_id from agent_queue_submission_origins where conversation_id=?1 and request_id=?2",params![intent.conversation_id.to_string(),request.to_string()],|row|row.get(0)).optional()?;
    let Some(origin) = origin else {
        return Ok(None);
    };
    if origin != intent.input_id.to_string() {
        return Err(CatalogError::Conflict.into());
    }
    let record = saved(db, intent.conversation_id, request)?;
    let expected = crate::agent::writing_submission::read_compaction_revision_in(
        db,
        intent.conversation_id,
        request,
    )
    .map_err(writing)?;
    let Some(expected_revision) = expected else {
        if pointer != Some(request) {
            return Ok(None);
        }
        return Ok(Some(QueuedCompactionRead::Submission {
            record,
            queue: queue(db, intent.conversation_id)?,
        }));
    };
    let (claim, frozen) = preparation(db, intent.conversation_id, intent.input_id, request)?
        .ok_or(CatalogError::Corrupt)?;
    if claim.manuscript_id != intent.manuscript_id
        || claim.expected_revision != expected_revision
        || record.input.text.trim() != "/compact"
    {
        return Err(CatalogError::Corrupt.into());
    }
    let current = pointer == Some(request) && latest.preparing() == Some(intent.input_id);
    let queue = if current { latest } else { frozen };
    let outcome = terminal(db, &record)?;
    Ok(Some(QueuedCompactionRead::Compact(QueuedCompactionState {
        record,
        queue,
        claim_revision: claim.expected_queue_revision,
        expected_revision,
        outcome,
        current,
        retirement: None,
    })))
}

/// The former queue dispatcher used R=I without a durable queue claim. Its
/// standalone Compact header cannot establish permission for a fresh attempt.
pub(super) fn reject_legacy_candidate(
    db: &Connection,
    conversation: Uuid,
    input: Uuid,
) -> Result<(), SqlMutationError> {
    if crate::agent::writing_submission::read_compaction_revision_in(db, conversation, input)
        .map_err(writing)?
        .is_some()
    {
        let record = saved(db, conversation, input)?;
        if record.input.text.trim() != "/compact" {
            return Err(CatalogError::Corrupt.into());
        }
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}

/// An explicit confirmation may execute its still-owned prepared claim after
/// startup paused the queue. It does not resume any following input or change Q.
pub(crate) fn require_startable_in(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<(), SqlMutationError> {
    let supported:bool=db.query_row("select exists(select 1 from sqlite_schema where type='table' and name='agent_queue_submission_origins')",[],|row|row.get(0))?;
    if !supported {
        return Ok(());
    }
    let input:Option<String>=db.query_row("select input_id from agent_queue_submission_origins where conversation_id=?1 and request_id=?2",params![conversation.to_string(),request.to_string()],|row|row.get(0)).optional()?;
    let Some(input) = input else { return Ok(()) };
    let input = Uuid::parse_str(&input).map_err(|_| CatalogError::Corrupt)?;
    let (claim, frozen) =
        preparation(db, conversation, input, request)?.ok_or(CatalogError::Conflict)?;
    let Some(QueuedCompactionRead::Compact(state)) = read_in(
        db,
        &ReadIntent {
            conversation_id: conversation,
            manuscript_id: claim.manuscript_id,
            input_id: input,
            request_id: Some(request),
        },
    )?
    else {
        return Err(CatalogError::Conflict.into());
    };
    let original = frozen
        .inputs()
        .iter()
        .find(|value| value.id == input)
        .ok_or(CatalogError::Corrupt)?;
    if !state.current
        || state.queue.inputs().iter().find(|value| value.id == input) != Some(original)
    {
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "legacy_tests.rs"]
mod legacy_tests;
