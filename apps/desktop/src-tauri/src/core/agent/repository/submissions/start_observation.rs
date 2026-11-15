//! Observe immutable admission facts without opening or recovering a session.
use research_agent_core::{
    conversation::ConversationId,
    protocol::{
        AgentEvent, ApplyResult, ConversationProjection, EventPayload, TaskKind, TurnOutcome,
    },
    submission::SubmissionRecord,
};
use rusqlite::Connection;

pub(crate) enum Error {
    Unavailable(rusqlite::Error),
    InvalidRow(rusqlite::Error),
    Invalid,
    IdentityMismatch,
    PrefixInvalid,
}

/// `Some(None)` proves admission even before its terminal. Rollback only hides
/// the visible turn; it cannot make an already accepted request fresh again.
pub(crate) fn observe(
    db: &Connection,
    record: &SubmissionRecord,
    expected_kind: TaskKind,
) -> Result<Option<Option<TurnOutcome>>, Error> {
    Ok(inspect(db, record, expected_kind)?.map(|observation| observation.outcome))
}

pub(crate) struct Observation {
    pub outcome: Option<TurnOutcome>,
    pub unresolved_tool_effect: bool,
}

pub(crate) fn inspect(
    db: &Connection,
    record: &SubmissionRecord,
    expected_kind: TaskKind,
) -> Result<Option<Observation>, Error> {
    let conversation = record.conversation_id.to_string();
    let count: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [&conversation],
            |row| row.get(0),
        )
        .map_err(Error::Unavailable)?;
    let mut statement = db
        .prepare("select ordinal,schema_version,payload_json from agent_items where conversation_id=?1 order by ordinal")
        .map_err(Error::Unavailable)?;
    let rows = statement
        .query_map([&conversation], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(Error::Unavailable)?;
    let mut projection =
        ConversationProjection::new(ConversationId::from_uuid(record.conversation_id));
    let mut started = false;
    let mut outcome = None;
    for row in rows {
        let (ordinal, schema, json) = row.map_err(Error::InvalidRow)?;
        let event: AgentEvent = serde_json::from_str(&json).map_err(|_| Error::Invalid)?;
        if schema != 1
            || ordinal != projection.seq()
            || projection.apply(&event).map_err(|_| Error::Invalid)? != ApplyResult::Applied
        {
            return Err(Error::Invalid);
        }
        if let EventPayload::TurnStarted {
            turn_id,
            context_snapshot_id,
            model,
            task_kind,
        } = event.payload()
            && (*turn_id == record.turn_id || event.request_id() == record.request_id)
        {
            if started
                || *turn_id != record.turn_id
                || event.request_id() != record.request_id
                || *context_snapshot_id != record.context_snapshot_id
                || *model != record.input.model
                || *task_kind != expected_kind
            {
                return Err(Error::IdentityMismatch);
            }
            started = true;
        }
        if let EventPayload::TurnFinished {
            turn_id,
            outcome: finished,
        } = event.payload()
            && *turn_id == record.turn_id
        {
            if !started || outcome.is_some() || event.request_id() != record.request_id {
                return Err(Error::IdentityMismatch);
            }
            outcome = Some(*finished);
        }
    }
    if projection.seq() != count {
        return Err(Error::PrefixInvalid);
    }
    Ok(started.then(|| Observation {
        outcome,
        unresolved_tool_effect: projection.has_unknown_tool_outcome(),
    }))
}
