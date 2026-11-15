//! History rewrites fence stale queue revisions without losing pending input.
use super::*;

/// Check the current queue in the TurnStarted journal transaction, never an old
/// admission receipt. Historical schemas predating queues have no queue origins.
pub(crate) fn require_startable_in(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<(), Error> {
    let has_origins: bool = db.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_queue_submission_origins')",
        [], |row| row.get(0),
    ).map_err(|_| Error::Storage)?;
    if !has_origins {
        return Ok(());
    }
    let queued: bool = db.query_row(
        "select exists(select 1 from agent_queue_submission_origins where conversation_id=?1 and request_id=?2)",
        params![conversation.to_string(), request.to_string()], |row| row.get(0),
    ).map_err(|_| Error::Storage)?;
    if queued && load(db, conversation)?.paused() {
        return Err(QueueError::Conflict.into());
    }
    Ok(())
}

/// Caller owns the journal transaction. Even an already-paused queue advances its
/// revision, so a frozen claim cannot confirm against the pre-rollback revision.
pub(crate) fn pause_in(db: &Connection, conversation: Uuid) -> Result<(), Error> {
    require_open(db, conversation)?;
    let queue = load(db, conversation)?;
    let next = queue.apply(queue.revision(), QueueEdit::Pause)?;
    db.execute(
        "insert into agent_input_queues(conversation_id,revision,snapshot_json) values(?1,?2,?3)
         on conflict(conversation_id) do update set revision=excluded.revision,snapshot_json=excluded.snapshot_json",
        params![conversation.to_string(), next.revision(), Snapshot::encode(&next)?],
    ).map_err(|_| Error::Storage)?;
    Ok(())
}
