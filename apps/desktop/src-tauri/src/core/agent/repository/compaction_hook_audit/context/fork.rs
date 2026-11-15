//! Fork trusted developer context with the journal and child window transaction.
//! Adapted from pinned Codex rollout fork/reconstruction (e363b08c9175ac1cbe5893615dd2cb9ddf95043b),
//! Apache-2.0, Copyright 2025 OpenAI. Snapshots do not claim hook re-execution.
use super::*;
use research_agent_core::conversation::CatalogError;
use std::collections::HashMap;

pub(crate) fn inherit(
    db: &Connection,
    parent: Uuid,
    child: Uuid,
    end: u64,
    windows: &HashMap<Uuid, Uuid>,
) -> Result<(), CatalogError> {
    let supported: bool = db.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_inherited_hook_contexts')",
        [], |row| row.get(0),
    ).map_err(|_| CatalogError::Unavailable)?;
    if !supported {
        return Ok(());
    }
    let history = context_windows::read_projection(db, child).map_err(|_| CatalogError::Corrupt)?;
    let mut bytes = 0usize;
    for (source, target) in windows {
        let mut query = db.prepare("select audit_seq,conversation_id,turn_id,boundary_seq,contexts_json from agent_compact_hook_contexts where conversation_id=?1 and window_id=?2 and boundary_seq<=?3 union all select source_audit_seq,source_conversation_id,turn_id,boundary_seq,contexts_json from agent_inherited_hook_contexts where conversation_id=?1 and window_id=?2 and boundary_seq<=?3 order by boundary_seq,audit_seq")
            .map_err(|_| CatalogError::Unavailable)?;
        let mut rows = query
            .query(params![parent.to_string(), source.to_string(), end])
            .map_err(|_| CatalogError::Unavailable)?;
        while let Some(row) = rows.next().map_err(|_| CatalogError::Unavailable)? {
            let audit: u64 = row.get(0).map_err(|_| CatalogError::Corrupt)?;
            let origin: String = row.get(1).map_err(|_| CatalogError::Corrupt)?;
            let turn: String = row.get(2).map_err(|_| CatalogError::Corrupt)?;
            let turn_id = Uuid::parse_str(&turn).map_err(|_| CatalogError::Corrupt)?;
            if history.turn(turn_id).is_none() {
                continue;
            }
            let boundary: u64 = row.get(3).map_err(|_| CatalogError::Corrupt)?;
            let encoded: String = row.get(4).map_err(|_| CatalogError::Corrupt)?;
            bytes = bytes
                .checked_add(encoded.len())
                .ok_or(CatalogError::TooLarge)?;
            if bytes > context_windows::MAX_CHECKPOINT_BYTES {
                return Err(CatalogError::TooLarge);
            }
            let contexts = serde_json::from_str(&encoded).map_err(|_| CatalogError::Corrupt)?;
            validate(&CompactionHookResult {
                outcome: CompactionHookOutcome::Continue,
                additional_contexts: contexts,
            })
            .map_err(|_| CatalogError::Corrupt)?;
            db.execute("insert into agent_inherited_hook_contexts(conversation_id,window_id,source_audit_seq,source_conversation_id,turn_id,boundary_seq,contexts_json) values(?1,?2,?3,?4,?5,?6,?7)",
                params![child.to_string(),target.to_string(),audit,origin,turn,boundary,encoded])
                .map_err(|_| CatalogError::Unavailable)?;
            crate::agent::repository::hook_spills::inherit(
                db,
                parent,
                child,
                (*source, *target),
                (audit, turn_id, boundary),
            )
            .map_err(|_| CatalogError::Unavailable)?;
        }
    }
    Ok(())
}
