//! Lightweight discovery only. Opening a copy still requires read_for_review
//! and native lease validation; a listed row grants no execution authority.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub created_at: String,
    pub published_proposal_id: Option<Uuid>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub next_cursor: Option<Uuid>,
}
/// Newest submissions first, with request identity breaking timestamp ties.
/// Caller keeps visibility checks and this query in one read transaction.
pub fn list(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    cursor: Option<Uuid>,
    limit: u32,
) -> Result<Page, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::Conflict);
    }
    let timestamp = cursor.map(|request| {
        db.query_row(
            "with w as (select conversation_id,request_id,manuscript_id from agent_execution_workspaces where conversation_id=?1 and manuscript_id=?2 union select conversation_id,request_id,manuscript_id from agent_workspace_discards where conversation_id=?1 and manuscript_id=?2 union select conversation_id,request_id,manuscript_id from agent_change_sets where conversation_id=?1 and manuscript_id=?2)
             select s.created_at from w join agent_submissions s on s.conversation_id=w.conversation_id and s.request_id=w.request_id where w.conversation_id=?1 and w.manuscript_id=?2 and w.request_id=?3",
            params![conversation.to_string(), manuscript.to_string(), request.to_string()],
            |row| row.get::<_, String>(0),
        ).optional()?.ok_or(Error::Conflict)
    }).transpose()?;
    let mut query = db.prepare(
        "with w as (select conversation_id,request_id,manuscript_id,turn_id,context_snapshot_id from agent_execution_workspaces where conversation_id=?1 and manuscript_id=?2 union select conversation_id,request_id,manuscript_id,turn_id,context_snapshot_id from agent_workspace_discards where conversation_id=?1 and manuscript_id=?2 union select conversation_id,request_id,manuscript_id,turn_id,context_snapshot_id from agent_change_sets where conversation_id=?1 and manuscript_id=?2)
         select w.request_id,w.turn_id,w.context_snapshot_id,s.created_at,
           (select p.id from agent_change_sets p where p.conversation_id=w.conversation_id
            and p.manuscript_id=w.manuscript_id and p.request_id=w.request_id
            and p.turn_id=w.turn_id and p.context_snapshot_id=w.context_snapshot_id
            and p.status in ('applied','undone') order by p.created_at desc,p.id desc limit 1)
         from w
         join agent_submissions s on s.conversation_id=w.conversation_id and s.request_id=w.request_id and s.turn_id=w.turn_id and s.context_snapshot_id=w.context_snapshot_id
         join agent_conversations c on c.id=w.conversation_id
         join manuscripts m on m.id=w.manuscript_id and m.status='active'
         where w.conversation_id=?1 and w.manuscript_id=?2
         and exists(select 1 from agent_conversation_bindings b where b.conversation_id=c.id and b.resource_kind='manuscript' and b.resource_id=m.id)
         and (?3 is null or s.created_at < ?3 or (s.created_at=?3 and w.request_id < ?4))
         order by s.created_at desc,w.request_id desc limit ?5",
    )?;
    let rows = query.query_map(
        params![
            conversation.to_string(),
            manuscript.to_string(),
            timestamp,
            cursor.map(|id| id.to_string()),
            limit + 1
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )?;
    let mut entries = Vec::new();
    for row in rows {
        let (request, turn, context, created_at, published) = row?;
        let id = |value: &str| {
            Uuid::parse_str(value)
                .ok()
                .filter(|id| !id.is_nil())
                .ok_or(Error::Conflict)
        };
        entries.push(Entry {
            request_id: id(&request)?,
            turn_id: id(&turn)?,
            context_snapshot_id: id(&context)?,
            created_at,
            published_proposal_id: published.as_deref().map(id).transpose()?,
        });
    }
    let more = entries.len() > limit as usize;
    entries.truncate(limit as usize);
    let next_cursor = if more {
        entries.last().map(|entry| entry.request_id)
    } else {
        None
    };
    Ok(Page {
        entries,
        next_cursor,
    })
}
