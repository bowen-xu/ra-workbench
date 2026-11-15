//! The mapping's frozen event count and pending metadata own this final phase.
use super::{AppStore, Error, SqliteConversationCatalog};
use research_agent_core::conversation::{ArchiveConversation, ConversationCatalog, ConversationId};
use uuid::Uuid;

pub(super) async fn apply(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    conversation: Uuid,
) -> Result<(), Error> {
    let (expected, status): (u64, String) = {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        db.query_row("select m.expected_event_count,l.status from agent_paper_reading_history_migrations m join agent_paper_reading_legacy_sessions l on l.conversation_id=m.conversation_id and l.paper_id=m.paper_id and l.legacy_session_id=m.legacy_session_id where m.conversation_id=?1 and m.status='pending'", [conversation.to_string()], |row| Ok((row.get(0)?,row.get(1)?)))
            .map_err(|_| Error::Corrupt)?
    };
    match status.as_str() {
        // Failed legacy sessions remain visible in the active catalog.
        "failed" => return Ok(()),
        "archived" => {}
        _ => return Err(Error::Corrupt),
    }
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v5(&conversation, b"agent.paper_migration.archive:v1"),
            id: ConversationId::from_uuid(conversation),
            archived: true,
            expected_watermark: Some(expected),
        })
        .await
        .map(|_| ())
        .map_err(|error| Error::Mutation(error.into()))
}
