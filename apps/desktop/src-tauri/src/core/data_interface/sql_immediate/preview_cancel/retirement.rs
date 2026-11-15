//! Immutable cancellation discovery under the caller's existing SQL/read fence.
use super::*;
use crate::agent::repository::input_queue::queued_compaction::{QueuedCompactionState, ReadIntent};
use rusqlite::params;

pub(crate) fn read_retirement_in(
    db: &Connection,
    command: &ReadIntent,
    state: &QueuedCompactionState,
    fence: ReadFence,
) -> Result<Option<Box<QueuePreviewRetirement>>, SqlImmediateFailure> {
    let request = state.record.request_id;
    let phase = "queue-retirement-read";
    let invalid =
        |reason| SqlImmediateFailure::Corrupt(Box::new(diagnostic(request, fence, phase, reason)));
    let mut statement = db.prepare(
        "select request_id,command_json from agent_input_queue_receipts
         where conversation_id=?1 and json_extract(command_json,'$[2][0]')='cancelPreview'
         and json_extract(command_json,'$[2][1]')=?2 and json_extract(command_json,'$[2][2]')=?3 limit 2",
    ).map_err(|cause| receipts::storage(request, fence, phase, &cause))?;
    let rows = statement
        .query_map(
            params![
                command.conversation_id.to_string(),
                command.input_id.to_string(),
                request.to_string()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|cause| receipts::storage(request, fence, phase, &cause))?;
    let candidates = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|cause| receipts::storage(request, fence, phase, &cause))?;
    let [(operation, raw)] = candidates.as_slice() else {
        return if candidates.is_empty() {
            Ok(None)
        } else {
            Err(invalid("queue_retirement_ambiguous"))
        };
    };
    let (version, revision, (kind, input, preview)): (u8, u64, (String, Uuid, Uuid)) =
        serde_json::from_str(raw).map_err(|_| invalid("queue_retirement_command_invalid"))?;
    if version != 1 || kind != "cancelPreview" || input != command.input_id || preview != request {
        return Err(invalid("queue_retirement_command_invalid"));
    }
    let intent = CancelPreviewIntent {
        conversation_id: command.conversation_id,
        manuscript_id: command.manuscript_id,
        request_id: Uuid::parse_str(operation)
            .map_err(|_| invalid("queue_retirement_operation_invalid"))?,
        expected_queue_revision: revision,
        input_id: input,
        preview_request_id: preview,
    };
    let identity = identity(&intent)?;
    receipts::check_cross_kind(db, intent.request_id, fence, phase)?;
    let receipt = receipts::lookup(db, &identity, fence, phase)?.ok_or_else(|| {
        SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
            intent.request_id,
            fence,
            phase,
            "queue_retirement_sql_proof_missing",
        )))
    })?;
    let queue = business(db, &intent)
        .map_err(|cause| error(&intent, fence, phase, cause))?
        .ok_or_else(|| invalid("queue_retirement_business_missing"))?;
    if state.current
        || state.outcome.is_some()
        || !frozen_scope(db, &intent).map_err(|cause| error(&intent, fence, phase, cause))?
        || !compact::check_prepare_proof(db, &intent, fence)?
    {
        return Err(invalid("queue_retirement_scope_invalid"));
    }
    Ok(Some(Box::new(QueuePreviewRetirement {
        intent,
        queue,
        receipt,
    })))
}
