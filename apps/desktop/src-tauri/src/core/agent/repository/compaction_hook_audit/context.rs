//! Persist context and hook completion atomically. Called by the task's existing
//! storage owner; no hook execution, model calls or second lifecycle lives here.
use super::*;
use crate::agent::repository::context_windows::{self, Error};
use research_agent_core::context::compaction_hooks::CompactionHookResult;
use rusqlite::TransactionBehavior;
use uuid::Uuid;

const MAX_CONTEXT_BYTES: usize = 1024 * 1024;
#[path = "context/fork.rs"]
pub(crate) mod fork;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContextBatch {
    pub turn_id: Uuid,
    pub boundary_seq: u64,
    pub contexts: Vec<String>,
}

fn validate(result: &CompactionHookResult) -> Result<(), Error> {
    if result.additional_contexts.len() > 256
        || result
            .additional_contexts
            .iter()
            .try_fold(0usize, |bytes, text| bytes.checked_add(text.len()))
            .is_none_or(|bytes| bytes > MAX_CONTEXT_BYTES)
        || (!result.additional_contexts.is_empty()
            && !matches!(
                result.outcome,
                CompactionHookOutcome::Continue | CompactionHookOutcome::Stop
            ))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn record_result(
    db: &mut Connection,
    request: &CompactionHookRequest,
    result: &CompactionHookResult,
    recorded_at_ms: i64,
) -> Result<(), Error> {
    record_result_with_handlers(db, request, result, recorded_at_ms, &[])
}

pub(crate) fn record_result_with_handlers(
    db: &mut Connection,
    request: &CompactionHookRequest,
    result: &CompactionHookResult,
    recorded_at_ms: i64,
    handlers: &[(Uuid, Option<CompactionHookOutcome>)],
) -> Result<(), Error> {
    validate(result)?;
    if request.stage != CompactionHookStage::SessionStartCompact
        && !result.additional_contexts.is_empty()
    {
        return Err(Error::Invalid);
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let boundary = if result.additional_contexts.is_empty() {
        None
    } else {
        let history = context_windows::read_projection(&tx, request.conversation_id)?;
        if history.active_turn() != Some(request.turn_id) {
            return Err(Error::Inactive);
        }
        let (count, archived): (u64, bool) = tx.query_row(
            "select durable_count,archived from agent_conversations where id=?1",
            [request.conversation_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if archived {
            return Err(Error::Inactive);
        }
        let active =
            context_windows::read_active(&tx, request.conversation_id)?.ok_or(Error::Conflict)?;
        if !context_windows::is_ancestor(
            &tx,
            request.conversation_id,
            active.window_id,
            request.operation_id,
        )? || history.seq() != count
        {
            return Err(Error::Conflict);
        }
        Some((history.seq(), active.window_id))
    };
    super::record(
        &tx,
        request,
        CompactionHookNotification::Completed(result.outcome),
        recorded_at_ms,
    )?;
    let audit_seq = tx.last_insert_rowid();
    super::handlers::record(&tx, audit_seq, handlers)?;
    if let Some((boundary, owner_window)) = boundary {
        let encoded =
            serde_json::to_string(&result.additional_contexts).map_err(|_| Error::Invalid)?;
        tx.execute("insert into agent_compact_hook_contexts(audit_seq,conversation_id,window_id,turn_id,boundary_seq,contexts_json) values(?1,?2,?3,?4,?5,?6)", params![audit_seq,request.conversation_id.to_string(),owner_window.to_string(),request.turn_id.to_string(),i64::try_from(boundary).map_err(|_| Error::Invalid)?,encoded])?;
        crate::agent::repository::hook_spills::activate(
            &tx,
            request,
            audit_seq,
            boundary,
            &result.additional_contexts,
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// A window-covered or rolled-back contributor is not injected again. Read
/// only through the caller's journal snapshot; keep original rows as evidence.
pub(crate) fn read_current(
    db: &Connection,
    conversation: Uuid,
) -> Result<Vec<ContextBatch>, Error> {
    let Some(window) = context_windows::read_active(db, conversation)? else {
        return Ok(vec![]);
    };
    let history = context_windows::read_projection(db, conversation)?;
    read(
        db,
        conversation,
        window.window_id,
        window.source_watermark,
        history.seq(),
    )
}

pub(crate) fn read(
    db: &Connection,
    conversation: Uuid,
    window: Uuid,
    after: u64,
    through: u64,
) -> Result<Vec<ContextBatch>, Error> {
    let history = context_windows::read_projection(db, conversation)?;
    let mut query = db.prepare("select turn_id,boundary_seq,contexts_json,audit_seq from agent_compact_hook_contexts where conversation_id=?1 and window_id=?2 and boundary_seq>?3 and boundary_seq<=?4 union all select turn_id,boundary_seq,contexts_json,source_audit_seq from agent_inherited_hook_contexts where conversation_id=?1 and window_id=?2 and boundary_seq>?3 and boundary_seq<=?4 order by boundary_seq,audit_seq")?;
    let mut rows = query.query(params![
        conversation.to_string(),
        window.to_string(),
        i64::try_from(after).map_err(|_| Error::Invalid)?,
        i64::try_from(through).map_err(|_| Error::Invalid)?
    ])?;
    let mut batches = vec![];
    let mut bytes = 0usize;
    while let Some(row) = rows.next()? {
        let encoded: String = row.get(2)?;
        bytes = bytes.checked_add(encoded.len()).ok_or(Error::Invalid)?;
        if bytes > context_windows::MAX_CHECKPOINT_BYTES {
            return Err(Error::Invalid);
        }
        let batch = ContextBatch {
            turn_id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|_| Error::Invalid)?,
            boundary_seq: row.get(1)?,
            contexts: serde_json::from_str(&encoded).map_err(|_| Error::Invalid)?,
        };
        validate(&CompactionHookResult {
            outcome: CompactionHookOutcome::Continue,
            additional_contexts: batch.contexts.clone(),
        })?;
        if batch.boundary_seq > history.seq() {
            return Err(Error::Invalid);
        }
        if history.turn(batch.turn_id).is_some() {
            batches.push(batch);
        }
    }
    Ok(batches)
}

#[cfg(test)]
#[path = "context/tests.rs"]
mod tests;
