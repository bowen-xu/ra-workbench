//! Durable initial-window contributor snapshots, adapted to the application's
//! immutable journal and rollback/fork projection. No external memory I/O here.
use super::*;
use research_agent_core::context::token_budget::TokenBudgetContext;

pub(crate) struct Snapshot {
    pub conversation: Uuid,
    pub window: Uuid,
    pub boundary: u64,
    pub turn: Uuid,
    pub body: String,
}
fn validate(snapshot: &Snapshot) -> Result<(), Error> {
    if snapshot.turn.is_nil()
        || snapshot.boundary == 0
        || snapshot.body.len() > MAX_CHECKPOINT_BYTES
    {
        return Err(Error::Invalid);
    }
    let metadata = TokenBudgetContext::from_body(&snapshot.body)
        .map_err(|_| Error::Invalid)?
        .ok_or(Error::Invalid)?;
    if metadata.conversation_id != snapshot.conversation
        || metadata.first_window_id != snapshot.window
        || metadata.window_id != snapshot.window
        || metadata.previous_window_id.is_some()
        || snapshot.window.get_version_num() != 7
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(crate) fn read(
    db: &Connection,
    conversation: Uuid,
    window: Uuid,
    through: u64,
) -> Result<Option<Snapshot>, Error> {
    let history = read_projection(db, conversation)?;
    let mut query = db.prepare("select boundary_seq,turn_id,body from agent_initial_window_metadata where conversation_id=?1 and window_id=?2 and boundary_seq<=?3 order by boundary_seq desc")?;
    let mut rows = query.query(params![
        conversation.to_string(),
        window.to_string(),
        i64::try_from(through).map_err(|_| Error::Invalid)?
    ])?;
    let mut bytes = 0usize;
    while let Some(row) = rows.next()? {
        let snapshot = Snapshot {
            conversation,
            window,
            boundary: row.get(0)?,
            turn: Uuid::parse_str(&row.get::<_, String>(1)?).map_err(|_| Error::Invalid)?,
            body: row.get(2)?,
        };
        bytes = bytes
            .checked_add(snapshot.body.len())
            .ok_or(Error::Invalid)?;
        if bytes > MAX_CHECKPOINT_BYTES {
            return Err(Error::Invalid);
        }
        validate(&snapshot)?;
        if history.turn(snapshot.turn).is_some() {
            return Ok(Some(snapshot));
        }
    }
    Ok(None)
}
pub(crate) fn record(
    db: &mut Connection,
    snapshot: &Snapshot,
    signal: &crate::agent::cancellation::AgentCancellation,
) -> Result<String, Error> {
    validate(snapshot)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let history = read_projection(&tx, snapshot.conversation)?;
    if history.active_turn() != Some(snapshot.turn) {
        return Err(Error::Inactive);
    }
    let (count, archived): (u64, bool) = tx.query_row(
        "select durable_count,archived from agent_conversations where id=?1",
        [snapshot.conversation.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if archived {
        return Err(Error::Inactive);
    }
    if count != history.seq() {
        return Err(Error::Conflict);
    }
    let root: String = tx.query_row(
        "select window_id from agent_initial_context_windows where conversation_id=?1",
        [snapshot.conversation.to_string()],
        |row| row.get(0),
    )?;
    let pointer: bool = tx.query_row(
        "select exists(select 1 from agent_active_context_windows where conversation_id=?1)",
        [snapshot.conversation.to_string()],
        |row| row.get(0),
    )?;
    if root != snapshot.window.to_string() || pointer {
        return Err(Error::Conflict);
    }
    if let Some(saved) = read(&tx, snapshot.conversation, snapshot.window, history.seq())? {
        return Ok(saved.body);
    }
    if snapshot.boundary != history.seq() {
        return Err(Error::Conflict);
    }
    tx.execute("insert into agent_initial_window_metadata(conversation_id,window_id,boundary_seq,turn_id,body) values(?1,?2,?3,?4,?5)", params![snapshot.conversation.to_string(),snapshot.window.to_string(),i64::try_from(snapshot.boundary).map_err(|_| Error::Invalid)?,snapshot.turn.to_string(),snapshot.body])?;
    signal
        .commit_if_active(|| tx.commit())
        .map_err(|_| Error::Canceled)??;
    Ok(snapshot.body.clone())
}

/// Caller owns the child publication transaction. Old database fixtures do not
/// have this table; they cannot have a snapshot to inherit.
pub(crate) fn inherit_for_fork(
    db: &Connection,
    parent: Uuid,
    root: Uuid,
    child: Uuid,
    child_root: Uuid,
    end: u64,
) -> Result<(), Error> {
    let supported: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_initial_window_metadata')", [], |row| row.get(0))?;
    if !supported {
        return Ok(());
    }
    let Some(snapshot) = read(db, parent, root, end)? else {
        return Ok(());
    };
    if read_projection(db, child)?.turn(snapshot.turn).is_none() {
        return Err(Error::Invalid);
    }
    let mut metadata = TokenBudgetContext::from_body(&snapshot.body)
        .map_err(|_| Error::Invalid)?
        .ok_or(Error::Invalid)?;
    metadata.conversation_id = child;
    metadata.first_window_id = child_root;
    metadata.window_id = child_root;
    let research_agent_core::model::ModelMessage::Developer(body) = metadata.into_message() else {
        return Err(Error::Invalid);
    };
    tx_insert(
        db,
        child,
        child_root,
        snapshot.boundary,
        snapshot.turn,
        &body,
    )
}
fn tx_insert(
    db: &Connection,
    conversation: Uuid,
    window: Uuid,
    boundary: u64,
    turn: Uuid,
    body: &str,
) -> Result<(), Error> {
    db.execute("insert into agent_initial_window_metadata(conversation_id,window_id,boundary_seq,turn_id,body) values(?1,?2,?3,?4,?5)", params![conversation.to_string(),window.to_string(),i64::try_from(boundary).map_err(|_| Error::Invalid)?,turn.to_string(),body])?;
    Ok(())
}

#[cfg(test)]
#[path = "initial_metadata/tests.rs"]
mod tests;
