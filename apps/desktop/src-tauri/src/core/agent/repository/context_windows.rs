//! Complete-window storage uses the existing journal and SQLite transaction owner.
//! Never start model I/O here or overwrite original conversation evidence.
use research_agent_core::{
    context::checkpoint::ContextCheckpoint,
    conversation::ConversationId,
    protocol::{AgentEvent, ConversationProjection},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

pub const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
#[path = "context_windows/fork.rs"]
mod fork;
#[path = "context_windows/identity.rs"]
pub(crate) mod identity;
#[path = "context_windows/initial_metadata.rs"]
pub(crate) mod initial_metadata;
#[path = "context_windows/install_managed.rs"]
pub(super) mod install_managed;
#[path = "context_windows/rollback.rs"]
mod rollback;
#[path = "context_windows/session_start.rs"]
pub(crate) mod session_start;
pub(super) use fork::inherit_for_fork;
pub(super) use rollback::apply_in as rollback_in;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("context checkpoint storage unavailable")]
    Storage,
    #[error("context checkpoint is invalid or corrupt")]
    Invalid,
    #[error("context checkpoint history or active window changed")]
    Conflict,
    #[error("context checkpoint task is no longer active")]
    Inactive,
    #[error("context checkpoint installation canceled")]
    Canceled,
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installation {
    Installed,
    /// A replay never reactivates a window superseded by another operation.
    AlreadyInstalled {
        active: bool,
    },
}

#[cfg(test)]
pub fn install(db: &mut Connection, checkpoint: &ContextCheckpoint) -> Result<Installation, Error> {
    install_owned(db, checkpoint, checkpoint.source_watermark, None)
}

#[cfg(test)]
pub(super) fn install_owned(
    db: &mut Connection,
    checkpoint: &ContextCheckpoint,
    expected_revision: u64,
    cancellation: Option<&crate::agent::cancellation::AgentCancellation>,
) -> Result<Installation, Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = install_in(&tx, checkpoint, expected_revision)?;
    if result == Installation::Installed {
        if let Some(cancellation) = cancellation {
            cancellation
                .commit_if_active(|| tx.commit())
                .map_err(|_| Error::Canceled)??;
        } else {
            tx.commit()?;
        }
    }
    Ok(result)
}

fn install_in(
    tx: &Connection,
    checkpoint: &ContextCheckpoint,
    expected_revision: u64,
) -> Result<Installation, Error> {
    checkpoint
        .validate(MAX_CHECKPOINT_BYTES)
        .map_err(|_| Error::Invalid)?;
    if checkpoint.source_watermark > expected_revision {
        return Err(Error::Invalid);
    }
    let encoded = serde_json::to_string(checkpoint).map_err(|_| Error::Invalid)?;
    let active: Option<String> = tx
        .query_row(
            "select window_id from agent_active_context_windows where conversation_id=?1",
            [checkpoint.conversation_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let existing: Option<(String, String)> = tx
        .query_row(
            "select conversation_id,checkpoint_json from agent_context_checkpoints where id=?1",
            [checkpoint.window_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((conversation, previous)) = existing {
        if conversation != checkpoint.conversation_id.to_string() || previous != encoded {
            return Err(Error::Conflict);
        }
        return Ok(Installation::AlreadyInstalled {
            active: active.as_deref() == Some(&checkpoint.window_id.to_string()),
        });
    }
    if active != checkpoint.previous_window_id.map(|id| id.to_string()) {
        return Err(Error::Conflict);
    }
    if let Some(active) = &active {
        let covered: u64 = tx.query_row(
            "select source_watermark from agent_context_checkpoints where id=?1",
            [active],
            |row| row.get(0),
        )?;
        if checkpoint.source_watermark < covered {
            return Err(Error::Conflict);
        }
    }
    let state: Option<(u64, bool)> = tx
        .query_row(
            "select durable_count,archived from agent_conversations where id=?1",
            [checkpoint.conversation_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((watermark, archived)) = state else {
        return Err(Error::Inactive);
    };
    if archived {
        return Err(Error::Inactive);
    }
    if watermark != expected_revision {
        return Err(Error::Conflict);
    }
    let projection = read_projection(tx, checkpoint.conversation_id)?;
    if projection.seq() != watermark
        || projection.active_turn() != Some(checkpoint.turn_id)
        || projection
            .turn(checkpoint.turn_id)
            .is_none_or(|turn| turn.model != checkpoint.binding.model)
    {
        return Err(Error::Inactive);
    }
    tx.execute("insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json) values(?1,?2,?3,?4,?5,?6,?7)", params![checkpoint.window_id.to_string(), checkpoint.conversation_id.to_string(), checkpoint.turn_id.to_string(), checkpoint.source_watermark, checkpoint.previous_window_id.map(|id| id.to_string()), checkpoint.schema_version, encoded])?;
    tx.execute("insert into agent_active_context_windows(conversation_id,window_id) values(?1,?2) on conflict(conversation_id) do update set window_id=excluded.window_id", params![checkpoint.conversation_id.to_string(), checkpoint.window_id.to_string()])?;
    session_start::enqueue(tx, checkpoint)?;
    Ok(Installation::Installed)
}

pub fn read_active(
    db: &Connection,
    conversation: Uuid,
) -> Result<Option<ContextCheckpoint>, Error> {
    if conversation.is_nil() {
        return Err(Error::Invalid);
    }
    let encoded: Option<(String, String, u32)> = db.query_row("select w.window_id,c.checkpoint_json,c.schema_version from agent_active_context_windows w join agent_context_checkpoints c on c.id=w.window_id and c.conversation_id=w.conversation_id where w.conversation_id=?1", [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    encoded
        .map(|(id, encoded, schema_version)| {
            if encoded.len() > MAX_CHECKPOINT_BYTES {
                return Err(Error::Invalid);
            }
            let checkpoint: ContextCheckpoint =
                serde_json::from_str(&encoded).map_err(|_| Error::Invalid)?;
            checkpoint
                .validate(MAX_CHECKPOINT_BYTES)
                .map_err(|_| Error::Invalid)?;
            if checkpoint.schema_version != schema_version
                || checkpoint.conversation_id != conversation
                || checkpoint.window_id.to_string() != id
            {
                return Err(Error::Invalid);
            }
            Ok(checkpoint)
        })
        .transpose()
}

pub(crate) fn is_ancestor(
    db: &Connection,
    conversation: Uuid,
    active: Uuid,
    candidate: Uuid,
) -> Result<bool, Error> {
    if conversation.is_nil() || active.is_nil() || candidate.is_nil() {
        return Err(Error::Invalid);
    }
    db.query_row(
        "with recursive ancestors(id) as (select ?2 union select c.previous_window_id from agent_context_checkpoints c join ancestors a on c.id=a.id where c.conversation_id=?1 and c.previous_window_id is not null) select exists(select 1 from ancestors where id=?3)",
        params![conversation.to_string(), active.to_string(), candidate.to_string()],
        |row| row.get(0),
    ).map_err(Into::into)
}

pub(crate) fn read_projection(
    db: &Connection,
    conversation: Uuid,
) -> Result<ConversationProjection, Error> {
    let mut projection = ConversationProjection::new(ConversationId::from_uuid(conversation));
    let mut query = db.prepare(
        "select payload_json from agent_items where conversation_id=?1 order by ordinal",
    )?;
    let mut rows = query.query([conversation.to_string()])?;
    while let Some(row) = rows.next()? {
        let encoded: String = row.get(0)?;
        let event: AgentEvent = serde_json::from_str(&encoded).map_err(|_| Error::Invalid)?;
        projection.apply(&event).map_err(|_| Error::Invalid)?;
    }
    Ok(projection)
}

#[cfg(test)]
#[path = "context_windows_tests.rs"]
mod tests;
