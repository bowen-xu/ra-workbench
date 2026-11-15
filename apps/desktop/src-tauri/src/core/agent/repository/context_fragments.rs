//! Trusted prompt records, not chat messages. Called under the existing session
//! owner on its blocking storage executor; this adapter never acquires a lease.
use super::context_windows::{self, Error};
use research_agent_core::protocol::{ConversationProjection, ItemKind};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

const MAX_FRAGMENT_BYTES: usize = 8192;

#[path = "context_fragments/fork.rs"]
mod fork;
#[path = "context_fragments/managed.rs"]
pub(super) mod managed;
pub(crate) use fork::inherit_for_fork;

#[derive(Clone, PartialEq, Eq)]
pub struct FragmentBatch {
    pub conversation_id: Uuid,
    pub window_id: Uuid,
    pub boundary_seq: u64,
    pub turn_id: Uuid,
    pub anchor_item_id: Uuid,
    /// Independent developer bodies, already resolved from trusted policy.
    pub fragments: Vec<String>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Recording {
    Recorded,
    AlreadyRecorded,
}

fn validate(batch: &FragmentBatch) -> Result<(), Error> {
    if [
        batch.conversation_id,
        batch.window_id,
        batch.turn_id,
        batch.anchor_item_id,
    ]
    .iter()
    .any(Uuid::is_nil)
        || batch.boundary_seq == 0
        || !(1..=2).contains(&batch.fragments.len())
        || batch.fragments.iter().any(|text| text.trim().is_empty())
        || batch
            .fragments
            .iter()
            .try_fold(0usize, |size, text| size.checked_add(text.len()))
            .is_none_or(|size| size > MAX_FRAGMENT_BYTES)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn anchor_valid(history: &ConversationProjection, batch: &FragmentBatch) -> bool {
    history.turn(batch.turn_id).is_some_and(|turn| {
        turn.task_kind.is_regular()
            && turn.items.get(&batch.anchor_item_id).is_some_and(|item| {
                item.completed
                    && matches!(
                        item.kind,
                        ItemKind::AssistantMessage | ItemKind::ToolResult { .. }
                    )
            })
    })
}

/// Caller holds the same owner lock through prepare, this transaction, and
/// runtime state commit. A replay never rewrites a record or reactivates a window.
#[cfg(test)]
pub fn record(db: &mut Connection, batch: &FragmentBatch) -> Result<Recording, Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = record_in(&tx, batch)?;
    if result == Recording::Recorded {
        tx.commit()?;
    }
    Ok(result)
}

fn record_in(tx: &Connection, batch: &FragmentBatch) -> Result<Recording, Error> {
    validate(batch)?;
    let seq = i64::try_from(batch.boundary_seq).map_err(|_| Error::Invalid)?;
    let history = context_windows::read_projection(tx, batch.conversation_id)?;
    if history.active_turn() != Some(batch.turn_id) || !anchor_valid(&history, batch) {
        return Err(Error::Inactive);
    }
    let (count, archived): (u64, bool) = tx.query_row(
        "select durable_count,archived from agent_conversations where id=?1",
        [batch.conversation_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if archived {
        return Err(Error::Inactive);
    }
    let encoded = serde_json::to_string(&batch.fragments).map_err(|_| Error::Invalid)?;
    let previous: Option<(String, String, String)> = tx.query_row(
        "select turn_id,anchor_item_id,fragments_json from agent_context_fragments where conversation_id=?1 and window_id=?2 and boundary_seq=?3",
        params![batch.conversation_id.to_string(), batch.window_id.to_string(), seq],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    if let Some(previous) = previous {
        if previous
            != (
                batch.turn_id.to_string(),
                batch.anchor_item_id.to_string(),
                encoded,
            )
        {
            return Err(Error::Conflict);
        }
        return Ok(Recording::AlreadyRecorded);
    }
    if count != batch.boundary_seq || history.seq() != count {
        return Err(Error::Conflict);
    }
    let active = context_windows::read_active(tx, batch.conversation_id)?;
    let has_pointer: bool = tx.query_row(
        "select exists(select 1 from agent_active_context_windows where conversation_id=?1)",
        [batch.conversation_id.to_string()],
        |row| row.get(0),
    )?;
    if has_pointer != active.is_some() {
        return Err(Error::Invalid);
    }
    let current = match active {
        Some(checkpoint) => checkpoint.window_id,
        None => {
            let initial: String = tx.query_row(
                "select window_id from agent_initial_context_windows where conversation_id=?1",
                [batch.conversation_id.to_string()],
                |row| row.get(0),
            )?;
            Uuid::parse_str(&initial).map_err(|_| Error::Invalid)?
        }
    };
    if current != batch.window_id {
        return Err(Error::Conflict);
    }
    let turn = history.turn(batch.turn_id).ok_or(Error::Inactive)?;
    let position = turn
        .item_order
        .iter()
        .position(|id| id == &batch.anchor_item_id)
        .ok_or(Error::Invalid)?;
    // A no-tools response may already have admitted pending steer before its
    // follow-up preparation. Keep the contributor before those user messages.
    if turn.item_order[position + 1..].iter().any(|id| {
        turn.items
            .get(id)
            .is_none_or(|item| !item.completed || !matches!(item.kind, ItemKind::UserMessage))
    }) {
        return Err(Error::Invalid);
    }
    // A fragment cannot split a pending tool batch.
    if turn.items.values().any(|item| match item.kind {
        ItemKind::ToolCall { call_id, .. } => !turn.items.values().any(|result| {
            matches!(result.kind, ItemKind::ToolResult { call_id: id, .. } if id == call_id)
                && result.completed
        }),
        _ => false,
    }) {
        return Err(Error::Invalid);
    }
    tx.execute("insert into agent_context_fragments(conversation_id,window_id,boundary_seq,turn_id,anchor_item_id,fragments_json) values(?1,?2,?3,?4,?5,?6)", params![batch.conversation_id.to_string(),batch.window_id.to_string(),seq,batch.turn_id.to_string(),batch.anchor_item_id.to_string(),encoded])?;
    Ok(Recording::Recorded)
}

/// Original records remain available as evidence. Rolled-back turns are omitted
/// by the authoritative projection, never inferred from current UI or settings.
pub fn read(
    db: &Connection,
    conversation: Uuid,
    window: Uuid,
    after: u64,
) -> Result<Vec<FragmentBatch>, Error> {
    let history = context_windows::read_projection(db, conversation)?;
    let mut query = db.prepare("select boundary_seq,turn_id,anchor_item_id,fragments_json from agent_context_fragments where conversation_id=?1 and window_id=?2 and boundary_seq>?3 order by boundary_seq")?;
    let mut rows = query.query(params![
        conversation.to_string(),
        window.to_string(),
        i64::try_from(after).map_err(|_| Error::Invalid)?
    ])?;
    let mut batches = vec![];
    let mut bytes = 0usize;
    while let Some(row) = rows.next()? {
        let encoded: String = row.get(3)?;
        // Bound decoding too; JSON escaping can expand each source byte sixfold.
        bytes = bytes.checked_add(encoded.len()).ok_or(Error::Invalid)?;
        if bytes > context_windows::MAX_CHECKPOINT_BYTES {
            return Err(Error::Invalid);
        }
        let batch = FragmentBatch {
            conversation_id: conversation,
            window_id: window,
            boundary_seq: row.get(0)?,
            turn_id: Uuid::parse_str(&row.get::<_, String>(1)?).map_err(|_| Error::Invalid)?,
            anchor_item_id: Uuid::parse_str(&row.get::<_, String>(2)?)
                .map_err(|_| Error::Invalid)?,
            fragments: serde_json::from_str(&encoded).map_err(|_| Error::Invalid)?,
        };
        validate(&batch)?;
        if batch.boundary_seq > history.seq() {
            return Err(Error::Invalid);
        }
        if history.turn(batch.turn_id).is_none() {
            continue;
        }
        if !anchor_valid(&history, &batch) {
            return Err(Error::Invalid);
        }
        batches.push(batch);
    }
    Ok(batches)
}

#[cfg(test)]
#[path = "context_fragments/tests.rs"]
mod tests;
