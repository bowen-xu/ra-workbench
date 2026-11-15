//! The existing host predicates, with SQLite evidence retained for the facade.
use super::*;
use crate::agent::repository::SqlRepositoryError;
use rusqlite::params;

pub(crate) fn authorize(
    db: &Connection,
    intent: &QueueEditIntent,
) -> Result<(), input_queue::QueueSqlError> {
    let conversation = intent.conversation_id.to_string();
    let (query, resource) = match &intent.scope {
        QueueEditScope::Context => {
            let allowed: bool = db.query_row("select exists(select 1 from agent_conversations c where c.id=?1 and c.archived=0 and not exists(select 1 from agent_conversation_bindings b where b.conversation_id=c.id and b.resource_kind='manuscript'))",
                [conversation], |row| row.get(0))?;
            return allowed
                .then_some(())
                .ok_or(SqlRepositoryError::Domain(input_queue::Error::Closed));
        }
        QueueEditScope::Manuscript { manuscript_id } => (
            "select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id join manuscripts m on m.id=b.resource_id where b.conversation_id=?1 and b.resource_kind='manuscript' and b.resource_id=?2 and c.archived=0 and m.status='active')",
            manuscript_id.to_string(),
        ),
        QueueEditScope::Paper { paper_id } => (
            "select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id join bibliographic_items p on p.id=b.resource_id where b.conversation_id=?1 and b.resource_kind='paper' and b.resource_id=?2 and c.archived=0 and p.trashed_at is null)",
            paper_id.clone(),
        ),
        QueueEditScope::Note { resource_id } => (
            "select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id join note_pages n on n.id=b.resource_id where b.conversation_id=?1 and b.resource_kind='note' and b.resource_id=?2 and c.archived=0 and n.status='active' and not exists(select 1 from reading_note_sources s where s.page_id=n.id))",
            resource_id.to_string(),
        ),
        QueueEditScope::ReadingNote { resource_id } => (
            "select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id join reading_note_sources s on s.block_id=b.resource_id join note_pages n on n.id=s.page_id where b.conversation_id=?1 and b.resource_kind='readingNote' and b.resource_id=?2 and c.archived=0 and n.status='active')",
            resource_id.to_string(),
        ),
    };
    let allowed: bool = db.query_row(query, params![conversation, resource], |row| row.get(0))?;
    allowed
        .then_some(())
        .ok_or(SqlRepositoryError::Domain(input_queue::Error::Closed))
}
