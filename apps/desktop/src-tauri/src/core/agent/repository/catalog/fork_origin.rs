//! A bounded, read-only parent lookup. Never infer lineage from titles or event IDs.
use super::*;
use research_agent_core::conversation::{ConversationForkOrigin, ConversationForkReader};
use research_agent_core::protocol::{AgentEvent, EventPayload};

impl ConversationForkReader for SqliteConversationCatalog {
    fn read_origin(
        &self,
        child: ConversationId,
        binding: ConversationBinding,
    ) -> CatalogFuture<'_, Option<ConversationForkOrigin>> {
        Box::pin(async move {
            if child.as_uuid().is_nil() {
                return Err(CatalogError::InvalidInput);
            }
            validate_binding(&binding)?;
            self.run(move |db| {
                let tx = db.transaction().map_err(db_error)?;
                let target = binding_target_query(binding.kind);
                let exists: bool = tx.query_row(target, [&binding.resource_id], |row| row.get(0)).map_err(db_error)?;
                if !exists { return Err(CatalogError::NotFound); }
                let child_summary = bound_summary(&tx, child, &binding)?;
                let row: Option<(String, String, i64, String, String)> = tx.query_row(
                    "select parent_id,through_turn_id,parent_end_seq,resource_kind,resource_id from agent_conversation_forks where child_id=?1",
                    [child.as_uuid().to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))
                ).optional().map_err(db_error)?;
                let Some((parent, turn, end, kind, resource)) = row else { return Ok(None); };
                if kind != binding_kind(binding.kind) || resource != binding.resource_id { return Err(CatalogError::NotFound); }
                let parent = ConversationId::from_uuid(Uuid::parse_str(&parent).map_err(|_| CatalogError::Corrupt)?);
                let turn = Uuid::parse_str(&turn).map_err(|_| CatalogError::Corrupt)?;
                if parent == child || parent.as_uuid().is_nil() || turn.is_nil() || end <= 0 { return Err(CatalogError::Corrupt); }
                let parent_summary = bound_summary(&tx, parent, &binding)?;
                let end = end as u64;
                if end > child_summary.durable_count || end > parent_summary.durable_count { return Err(CatalogError::Corrupt); }
                // Read only the indexed boundary events, not an unbounded transcript.
                validate_boundary(&tx, child, turn, end)?;
                validate_boundary(&tx, parent, turn, end)?;
                Ok(Some(ConversationForkOrigin { parent: parent_summary, through_turn_id: turn, parent_end_seq: end }))
            }).await
        })
    }
}
fn bound_summary(
    db: &Connection,
    id: ConversationId,
    binding: &ConversationBinding,
) -> Result<ConversationSummary, CatalogError> {
    let row = db.query_row(
        "select id,title,archived,durable_count,created_at,updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=c.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=c.id)) from agent_conversations c where id=?1 and exists
         (select 1 from agent_conversation_bindings b where b.conversation_id=c.id and b.resource_kind=?2 and b.resource_id=?3)",
        params![id.as_uuid().to_string(),binding_kind(binding.kind),binding.resource_id], read_row,
    ).optional().map_err(db_error)?.ok_or(CatalogError::NotFound)?;
    decode(row)
}
fn validate_boundary(
    db: &Connection,
    id: ConversationId,
    turn: Uuid,
    end: u64,
) -> Result<(), CatalogError> {
    // A terminal envelope is small; corrupt oversized content must not become an unbounded read.
    let json: Option<String> = db.query_row("select case when length(cast(payload_json as blob))<=4096 then payload_json else null end from agent_items where conversation_id=?1 and ordinal=?2",
        params![id.as_uuid().to_string(),end-1], |row| row.get(0)).optional().map_err(db_error)?.flatten();
    let json = json.ok_or(CatalogError::Corrupt)?;
    let event: AgentEvent = serde_json::from_str(&json).map_err(|_| CatalogError::Corrupt)?;
    if event.conversation_id() != id
        || event.seq() != end
        || !matches!(event.payload(), EventPayload::TurnFinished { turn_id, .. } if *turn_id == turn)
    {
        return Err(CatalogError::Corrupt);
    }
    Ok(())
}
