//! Read-only, transactionally consistent transcript export. Never opens a runtime owner.
use super::*;
use research_agent_core::{
    conversation::ConversationExport,
    protocol::{AgentEvent, ConversationProjection},
};

const MAX_EXPORT_EVENTS: i64 = 200_000;
const MAX_EXPORT_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotExportError {
    #[error("conversation not found")]
    NotFound,
    #[error("conversation exceeds the transcript export budget")]
    TooLarge,
    #[error("conversation history is corrupt or unsupported")]
    Corrupt,
    #[error("conversation export unavailable")]
    Unavailable,
}

impl SqliteConversationCatalog {
    /// Includes only durable normalized history, even when a turn is running or
    /// the conversation is archived. No credentials, context expansion, files,
    /// opaque provider state, permissions or process journals are loaded.
    pub async fn export_snapshot(
        &self,
        id: ConversationId,
    ) -> Result<ConversationExport, SnapshotExportError> {
        self.snapshot_with(id, project_snapshot).await
    }

    /// Native consumers share the export reader's extent, ordinal and transition
    /// validation. This is a read snapshot, not a runtime owner or authorization.
    pub(crate) async fn projection_snapshot(
        &self,
        id: ConversationId,
    ) -> Result<(ConversationSummary, ConversationProjection), SnapshotExportError> {
        self.snapshot_with(id, project_history).await
    }

    async fn snapshot_with<T: Send + 'static>(
        &self,
        id: ConversationId,
        project: fn((ConversationSummary, Vec<AgentEvent>)) -> Result<T, SnapshotExportError>,
    ) -> Result<T, SnapshotExportError> {
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || {
                let saved = {
                    let mut db = connection
                        .lock()
                        .map_err(|_| SnapshotExportError::Unavailable)?;
                    read_snapshot(&mut db, id)?
                };
                // Projection/serialization work must not retain the shared database lock.
                project(saved)
            })
            .await
            .map_err(|_| SnapshotExportError::Unavailable)?
    }
}

pub(super) fn read_snapshot(
    db: &mut Connection,
    id: ConversationId,
) -> Result<(ConversationSummary, Vec<AgentEvent>), SnapshotExportError> {
    use SnapshotExportError::{Corrupt, NotFound, Unavailable};
    let tx = db.transaction().map_err(|_| Unavailable)?;
    let key = id.as_uuid().to_string();
    let summary = tx.query_row(
        "select id,title,archived,durable_count,created_at,updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) from agent_conversations where id=?1",
        [&key], read_row).optional().map_err(|_| Unavailable)?.ok_or(NotFound)?;
    let summary = decode(summary).map_err(|_| Corrupt)?;
    let (count, bytes): (i64, i64) = tx.query_row(
        "select count(*),coalesce(sum(length(cast(payload_json as blob))),0) from agent_items where conversation_id=?1",
        [&key], |row| Ok((row.get(0)?, row.get(1)?))).map_err(|_| Unavailable)?;
    validate_extent(count, bytes, summary.durable_count)?;
    let mut events = Vec::new();
    {
        let mut statement = tx.prepare(
            "select ordinal,schema_version,payload_json from agent_items where conversation_id=?1 order by ordinal"
        ).map_err(|_| Unavailable)?;
        let mut rows = statement.query([&key]).map_err(|_| Unavailable)?;
        while let Some(row) = rows.next().map_err(|_| Unavailable)? {
            let ordinal: i64 = row.get(0).map_err(|_| Corrupt)?;
            let version: i64 = row.get(1).map_err(|_| Corrupt)?;
            if version != 1 || ordinal < 0 || ordinal as usize != events.len() {
                return Err(Corrupt);
            }
            let json: String = row.get(2).map_err(|_| Corrupt)?;
            let event: AgentEvent = serde_json::from_str(&json).map_err(|_| Corrupt)?;
            if event.seq() != events.len() as u64 + 1 || event.conversation_id() != id {
                return Err(Corrupt);
            }
            events.push(event);
        }
    }
    tx.commit().map_err(|_| Unavailable)?;
    Ok((summary, events))
}

fn project_snapshot(
    saved: (ConversationSummary, Vec<AgentEvent>),
) -> Result<ConversationExport, SnapshotExportError> {
    let (summary, projection) = project_history(saved)?;
    ConversationExport::from_projection(summary, &projection)
        .map_err(|_| SnapshotExportError::Corrupt)
}

fn project_history(
    (summary, events): (ConversationSummary, Vec<AgentEvent>),
) -> Result<(ConversationSummary, ConversationProjection), SnapshotExportError> {
    let mut projection = ConversationProjection::new(summary.id);
    for event in events {
        projection
            .apply(&event)
            .map_err(|_| SnapshotExportError::Corrupt)?;
    }
    if projection.seq() != summary.durable_count {
        return Err(SnapshotExportError::Corrupt);
    }
    Ok((summary, projection))
}

fn validate_extent(count: i64, bytes: i64, watermark: u64) -> Result<(), SnapshotExportError> {
    if count < 0 || bytes < 0 || count as u64 != watermark {
        return Err(SnapshotExportError::Corrupt);
    }
    if count > MAX_EXPORT_EVENTS || bytes > MAX_EXPORT_BYTES {
        return Err(SnapshotExportError::TooLarge);
    }
    Ok(())
}

#[cfg(test)]
#[path = "export/tests.rs"]
mod tests;
