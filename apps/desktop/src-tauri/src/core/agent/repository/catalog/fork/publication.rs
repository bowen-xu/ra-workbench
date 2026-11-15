//! The facade owns the transaction; this body publishes the entire historical child.
use super::*;
use crate::data_interface::SqlMutationError;
use rusqlite::Transaction;

pub(crate) struct PreparedFork {
    pub(super) request: ConversationForkRequest,
    pub(super) summary: ConversationSummary,
    pub(super) end: u64,
    pub(super) encoded: Vec<String>,
    pub(super) turns: Vec<(Uuid, Uuid)>,
}
impl PreparedFork {
    /// Validate and encode a complete exported parent snapshot off the SQL lock.
    pub(crate) fn prepare(
        request: ConversationForkRequest,
        summary: ConversationSummary,
        events: &[research_agent_core::protocol::AgentEvent],
    ) -> Result<Self, CatalogError> {
        validate_fork_request(&request)?;
        if summary.id != request.parent_id || summary.durable_count != events.len() as u64 {
            return Err(CatalogError::Corrupt);
        }
        let history = materialize_fork_history(&request, events)?;
        let end = history.len() as u64;
        let turns = history
            .iter()
            .filter_map(|event| match event.payload() {
                research_agent_core::protocol::EventPayload::TurnStarted {
                    turn_id,
                    context_snapshot_id,
                    ..
                } => Some((*turn_id, *context_snapshot_id)),
                _ => None,
            })
            .collect();
        let encoded = history
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| CatalogError::Corrupt)?;
        Ok(Self {
            request,
            summary,
            end,
            turns,
            encoded,
        })
    }
    pub(crate) fn request(&self) -> &ConversationForkRequest {
        &self.request
    }
}
pub(crate) fn fork_in(
    tx: &Transaction<'_>,
    prepared: &PreparedFork,
) -> Result<(), SqlMutationError> {
    let request = &prepared.request;
    // A legacy row is readable, but cannot be retroactively certified as a new
    // managed commit. Concurrent managed publications have already replayed proof.
    if replay(tx, request)?.is_some() {
        return Err(CatalogError::Conflict.into());
    }
    let parent = request.parent_id.as_uuid().to_string();
    let child = request.child_id.as_uuid().to_string();
    let bound: bool = tx.query_row(
        "select exists(select 1 from agent_conversation_bindings where conversation_id=?1 and resource_kind=?2 and resource_id=?3)",
        params![parent,binding_kind(request.binding.kind),request.binding.resource_id],|row|row.get(0),
    )?;
    if !bound
        || !tx.query_row(
            binding_target_query(request.binding.kind),
            [&request.binding.resource_id],
            |row| row.get::<_, bool>(0),
        )?
    {
        return Err(CatalogError::NotFound.into());
    }
    let count: i64 = tx
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [&parent],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(CatalogError::NotFound)?;
    if u64::try_from(count).ok() != Some(prepared.summary.durable_count) {
        return Err(CatalogError::Conflict.into());
    }
    let title = format!(
        "{} · 分支",
        prepared.summary.title.chars().take(175).collect::<String>()
    );
    tx.execute(
        "insert into agent_conversations(id,title,durable_count) values(?1,?2,?3)",
        params![child, title, prepared.end],
    )?;
    tx.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,?2,?3)",
        params![child,binding_kind(request.binding.kind),request.binding.resource_id])?;
    for (ordinal, event) in prepared.encoded.iter().enumerate() {
        tx.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)",params![child,ordinal as u64,event])?;
    }
    super::super::super::search_visibility::rebuild_in(tx, request.child_id)?;
    for (turn, snapshot) in &prepared.turns {
        tx.execute("insert into agent_fork_turn_sources(conversation_id,turn_id,source_conversation_id,context_snapshot_id)
            values(?1,?2,coalesce((select source_conversation_id from agent_fork_turn_sources where conversation_id=?3 and turn_id=?2),?3),?4)",
            params![child,turn.to_string(),parent,snapshot.to_string()])?;
    }
    super::super::super::context_windows::inherit_for_fork(
        tx,
        *request.parent_id.as_uuid(),
        *request.child_id.as_uuid(),
        prepared.end,
    )?;
    tx.execute("insert into agent_conversation_forks(child_id,parent_id,through_turn_id,parent_end_seq,resource_kind,resource_id) values(?1,?2,?3,?4,?5,?6)",
        params![child,parent,request.through_turn_id.to_string(),prepared.end,binding_kind(request.binding.kind),request.binding.resource_id])?;
    Ok(())
}
