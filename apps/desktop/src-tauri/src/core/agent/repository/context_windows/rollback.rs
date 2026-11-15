//! Rollback marker and surviving replacement pointer share the journal transaction.
use super::super::JournalError;
use super::*;
use std::collections::HashSet;

pub(crate) fn apply_in(db: &Connection, event: &AgentEvent) -> Result<(), JournalError> {
    let id = *event.conversation_id().as_uuid();
    let mut history = read_projection(db, id).map_err(map_error)?;
    let before = history.turn_order().to_vec();
    history
        .apply(event)
        .map_err(|_| JournalError::InvalidInput)?;
    super::super::search_visibility::record_removed_in(db, event, &before, &history)?;
    let mut current = read_active(db, id)
        .map_err(map_error)?
        .map(|checkpoint| {
            fork::read_previous(db, id, checkpoint.window_id).map_err(|_| JournalError::Corrupt)
        })
        .transpose()?;
    let mut selected = None;
    let mut seen = HashSet::new();
    let mut newer = u64::MAX;
    let mut bytes = 0usize;
    while let Some(checkpoint) = current {
        if !seen.insert(checkpoint.window_id) || checkpoint.source_watermark > newer {
            return Err(JournalError::Corrupt);
        }
        newer = checkpoint.source_watermark;
        bytes = bytes
            .checked_add(
                serde_json::to_vec(&checkpoint)
                    .map_err(|_| JournalError::Corrupt)?
                    .len(),
            )
            .ok_or(JournalError::Corrupt)?;
        if bytes > MAX_CHECKPOINT_BYTES {
            return Err(JournalError::Corrupt);
        }
        if let Some(turn) = history.turn(checkpoint.turn_id) {
            if turn.model != checkpoint.binding.model || checkpoint.source_watermark >= event.seq()
            {
                return Err(JournalError::Corrupt);
            }
            history
                .context_tail(checkpoint.source_watermark)
                .map_err(|_| JournalError::Corrupt)?;
            selected.get_or_insert(checkpoint.window_id);
        }
        current = checkpoint
            .previous_window_id
            .map(|previous| {
                fork::read_previous(db, id, previous).map_err(|_| JournalError::Corrupt)
            })
            .transpose()?;
    }
    if let Some(window) = selected {
        db.execute(
            "update agent_active_context_windows set window_id=?2 where conversation_id=?1",
            params![id.to_string(), window.to_string()],
        )?;
    } else {
        db.execute(
            "delete from agent_active_context_windows where conversation_id=?1",
            [id.to_string()],
        )?;
    }
    session_start::clear(db, id).map_err(map_error)?;
    Ok(())
}

fn map_error(error: Error) -> JournalError {
    match error {
        Error::Storage => JournalError::Unavailable,
        _ => JournalError::Corrupt,
    }
}

#[cfg(test)]
#[path = "rollback/queue_tests.rs"]
mod queue_tests;
#[cfg(test)]
#[path = "rollback/tests.rs"]
mod tests;
