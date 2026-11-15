//! Discover an existing intent without requiring the UI to remember its operation
//! ID. This read-only lookup grants no execution or filesystem deletion authority.
use super::*;

pub(super) fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Request, String)> {
    let id = |column| {
        Uuid::parse_str(&r.get::<_, String>(column)?).map_err(|_| rusqlite::Error::InvalidQuery)
    };
    Ok((
        Request {
            operation_id: id(0)?,
            binding: Binding {
                conversation_id: id(1)?,
                request_id: id(2)?,
                turn_id: id(3)?,
                context_snapshot_id: id(4)?,
                manuscript_id: id(5)?,
                lease_id: id(6)?,
                manifest_hash: r.get(8)?,
            },
            environment_id: r.get(7)?,
            inventory_hash: r.get(9)?,
        },
        r.get(10)?,
    ))
}

/// Caller supplies one storage snapshot. Archived conversations remain eligible,
/// but a current manuscript binding and the immutable submission are required.
/// Exact completion retry after deletion is deliberately a separate native API.
pub fn find(
    db: &Connection,
    conversation: Uuid,
    submission: Uuid,
    manuscript: Uuid,
    context: Uuid,
) -> Result<Option<(Request, State)>, Error> {
    if [conversation, submission, manuscript, context]
        .iter()
        .any(Uuid::is_nil)
    {
        return Err(Error::Conflict);
    }
    let visible: bool = db.query_row(
        "select exists(select 1 from agent_submissions s join agent_conversations c on c.id=s.conversation_id join agent_conversation_bindings b on b.conversation_id=c.id join manuscripts m on m.id=b.resource_id where s.conversation_id=?1 and s.request_id=?2 and s.context_snapshot_id=?3 and b.resource_kind='manuscript' and b.resource_id=?4 and m.status='active')",
        params![conversation.to_string(), submission.to_string(), context.to_string(), manuscript.to_string()],
        |r| r.get(0),
    )?;
    if !visible {
        return Err(Error::Denied);
    }
    let saved = db.query_row(
        "select operation_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,environment_id,manifest_hash,inventory_hash,state from agent_workspace_discards where conversation_id=?1 and request_id=?2",
        params![conversation.to_string(), submission.to_string()], decode,
    ).optional()?;
    let Some((request, _)) = saved else {
        return Ok(None);
    };
    request.validate()?;
    if request.binding.manuscript_id != manuscript || request.binding.context_snapshot_id != context
    {
        return Err(Error::Conflict);
    }
    let turn: String = db.query_row(
        "select turn_id from agent_submissions where conversation_id=?1 and request_id=?2",
        params![conversation.to_string(), submission.to_string()],
        |r| r.get(0),
    )?;
    if turn != request.binding.turn_id.to_string() {
        return Err(Error::Conflict);
    }
    let state = read(db, &request)?.ok_or(Error::Conflict)?;
    Ok(Some((request, state)))
}
