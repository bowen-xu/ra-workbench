//! Search-only metadata derived from the authoritative Core rollback projection.
//! Bodies, native facts and immutable journal positions remain untouched.
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, ApplyResult, ConversationProjection, EventPayload},
};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

fn supported(db: &Connection) -> rusqlite::Result<bool> {
    let exists: bool = db.query_row(
        "select exists(select 1 from sqlite_schema where type='table' and name='agent_rolled_back_turns')",
        [], |row| row.get(0),
    )?;
    if !exists {
        let version: i64 = db.query_row(
            "select coalesce(max(version),0) from schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version >= 212 {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    Ok(exists)
}

/// Called after Core accepts the marker, within the same journal transaction.
/// Older deployed schemas must drain their pending log before 212 is installed.
pub(super) fn record_removed_in(
    db: &Connection,
    marker: &AgentEvent,
    before: &[Uuid],
    after: &ConversationProjection,
) -> rusqlite::Result<()> {
    if db.is_autocommit() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if !supported(db)? {
        return Ok(());
    }
    if !matches!(
        marker.payload(),
        EventPayload::ConversationRolledBack { .. }
    ) || marker.conversation_id() != after.conversation_id()
        || marker.seq() != after.seq()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let conversation = marker.conversation_id().as_uuid().to_string();
    let ordinal = marker
        .seq()
        .checked_sub(1)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    for id in before.iter().filter(|id| after.turn(**id).is_none()) {
        let turn = id.to_string();
        let previous: Option<u64> = db.query_row(
            "select rollback_ordinal from agent_rolled_back_turns where conversation_id=?1 and turn_id=?2",
            params![conversation, turn], |row| row.get(0),
        ).optional()?;
        match previous {
            Some(saved) if saved == ordinal => {}
            Some(_) => return Err(rusqlite::Error::InvalidQuery),
            None => {
                db.execute(
                "insert into agent_rolled_back_turns(conversation_id,turn_id,rollback_ordinal) values(?1,?2,?3)",
                params![conversation, turn, ordinal],
            )?;
            }
        }
    }
    Ok(())
}

/// Used by migration and fork publication, never by a search query. Only the
/// prefix through the final marker is decoded; later large history is irrelevant.
pub(super) fn rebuild_in(db: &Connection, conversation: ConversationId) -> rusqlite::Result<()> {
    if db.is_autocommit() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if !supported(db)? {
        return Ok(());
    }
    let id = conversation.as_uuid().to_string();
    let last: Option<u64> = db.query_row(
        "select max(ordinal) from agent_journal_records where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='conversationRolledBack'",
        [&id], |row| row.get(0),
    )?;
    let Some(last) = last else {
        return Ok(());
    };
    let mut state = ConversationProjection::new(conversation);
    let mut query = db.prepare(
        "select ordinal,payload_json from agent_items where conversation_id=?1 and ordinal<=?2 order by ordinal",
    )?;
    let mut rows = query.query(params![id, last])?;
    while let Some(row) = rows.next()? {
        let ordinal: u64 = row.get(0)?;
        let encoded: String = row.get(1)?;
        let event: AgentEvent =
            serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
        if event.seq().checked_sub(1) != Some(ordinal) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let before = matches!(event.payload(), EventPayload::ConversationRolledBack { .. })
            .then(|| state.turn_order().to_vec());
        if state
            .apply(&event)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            != ApplyResult::Applied
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if let Some(before) = before {
            record_removed_in(db, &event, &before, &state)?;
        }
    }
    if state.seq().checked_sub(1) != Some(last) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

/// Runs inside migration 212 before its version and checksum are committed.
pub(crate) fn backfill_in(db: &Connection) -> rusqlite::Result<()> {
    if db.is_autocommit() || !supported(db)? {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut query = db.prepare(
        "select distinct conversation_id from agent_journal_records where json_extract(payload_json,'$.payload.type')='conversationRolledBack' order by conversation_id",
    )?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let id = Uuid::parse_str(&id).map_err(|_| rusqlite::Error::InvalidQuery)?;
        rebuild_in(db, ConversationId::from_uuid(id))?;
    }
    Ok(())
}
