//! Fork replacement history along the installed chain, never wall-clock order.
//! Adapted from pinned Codex compact_resume_fork.rs / rollout reconstruction:
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0, Copyright 2025 OpenAI.
use super::*;
use research_agent_core::conversation::CatalogError;
use std::collections::{HashMap, HashSet};

/// Caller owns the same transaction that publishes the child journal and lineage.
pub(crate) fn inherit_for_fork(
    db: &Connection,
    parent: Uuid,
    child: Uuid,
    end: u64,
) -> Result<(), CatalogError> {
    // Pre-window database fixtures/exports have no replacement state to inherit.
    let supported: bool = db.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_context_checkpoints')",
        [], |row| row.get(0),
    ).map_err(|_| CatalogError::Unavailable)?;
    if !supported {
        return Ok(());
    }
    let mut current = read_active(db, parent)
        .map_err(map_error)?
        .map(|checkpoint| read_previous(db, parent, checkpoint.window_id))
        .transpose()?;
    let history = read_projection(db, child).map_err(map_error)?;
    if history.seq() != end || history.active_turn().is_some() {
        return Err(CatalogError::Corrupt);
    }
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    let mut bytes = 0usize;
    let mut newer_watermark = u64::MAX;
    while let Some(checkpoint) = current {
        if !seen.insert(checkpoint.window_id) || checkpoint.source_watermark > newer_watermark {
            return Err(CatalogError::Corrupt);
        }
        newer_watermark = checkpoint.source_watermark;
        bytes = bytes
            .checked_add(
                serde_json::to_vec(&checkpoint)
                    .map_err(|_| CatalogError::Corrupt)?
                    .len(),
            )
            .ok_or(CatalogError::TooLarge)?;
        if bytes > MAX_CHECKPOINT_BYTES {
            return Err(CatalogError::TooLarge);
        }
        if let Some(turn) = history.turn(checkpoint.turn_id) {
            if checkpoint.source_watermark > end || checkpoint.binding.model != turn.model {
                return Err(CatalogError::Corrupt);
            }
            history
                .context_tail(checkpoint.source_watermark)
                .map_err(|_| CatalogError::Corrupt)?;
            selected.push(checkpoint.clone());
        }
        current = checkpoint
            .previous_window_id
            .map(|id| read_previous(db, parent, id))
            .transpose()?;
    }
    let window_ids: HashMap<_, _> = selected
        .iter()
        .map(|checkpoint| (checkpoint.window_id, Uuid::new_v4()))
        .collect();
    // Allocate the child root and copy contributors in the same publication
    // transaction before rebuilding checkpoint metadata with child identities.
    super::super::context_fragments::inherit_for_fork(db, parent, child, end, window_ids.clone())?;
    let mut previous = None;
    for mut checkpoint in selected.into_iter().rev() {
        let source_id = checkpoint.window_id;
        rebind_metadata(db, &mut checkpoint, child, window_ids[&source_id], previous)?;
        checkpoint.conversation_id = child;
        checkpoint.window_id = window_ids[&source_id];
        checkpoint.previous_window_id = previous;
        checkpoint
            .validate(MAX_CHECKPOINT_BYTES)
            .map_err(|_| CatalogError::Corrupt)?;
        let encoded = serde_json::to_string(&checkpoint).map_err(|_| CatalogError::Corrupt)?;
        db.execute(
            "insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json) values(?1,?2,?3,?4,?5,?6,?7)",
            params![checkpoint.window_id.to_string(), child.to_string(), checkpoint.turn_id.to_string(),
                checkpoint.source_watermark, previous.map(|id: Uuid| id.to_string()), checkpoint.schema_version, encoded],
        ).map_err(|_| CatalogError::Unavailable)?;
        previous = Some(checkpoint.window_id);
    }
    super::super::compaction_hook_audit::context::fork::inherit(
        db,
        parent,
        child,
        end,
        &window_ids,
    )?;
    if let Some(id) = previous {
        db.execute(
            "insert into agent_active_context_windows(conversation_id,window_id) values(?1,?2)",
            params![child.to_string(), id.to_string()],
        )
        .map_err(|_| CatalogError::Unavailable)?;
    }
    Ok(())
}

fn rebind_metadata(
    db: &Connection,
    checkpoint: &mut ContextCheckpoint,
    child: Uuid,
    current: Uuid,
    previous: Option<Uuid>,
) -> Result<(), CatalogError> {
    use research_agent_core::context::{
        checkpoint::StoredMessage, token_budget::TokenBudgetContext,
    };
    let source_conversation = checkpoint.conversation_id;
    let source_window = checkpoint.window_id;
    let source_previous = checkpoint.previous_window_id;
    let Some(canonical) = checkpoint.canonical_context.as_mut() else {
        return Ok(());
    };
    let mut found = false;
    for message in &mut canonical.messages {
        let StoredMessage::Developer { text } = message else {
            continue;
        };
        let Some(mut metadata) =
            TokenBudgetContext::from_body(text).map_err(|_| CatalogError::Corrupt)?
        else {
            continue;
        };
        if found
            || metadata.conversation_id != source_conversation
            || metadata.window_id != source_window
        {
            return Err(CatalogError::Corrupt);
        }
        found = true;
        let root =
            |conversation: Uuid| -> Result<Uuid, CatalogError> {
                let id: String = db.query_row(
                "select window_id from agent_initial_context_windows where conversation_id=?1",
                [conversation.to_string()], |row| row.get(0),
            ).map_err(|_| CatalogError::Corrupt)?;
                Uuid::parse_str(&id)
                    .ok()
                    .filter(|id| !id.is_nil() && id.get_version_num() == 7)
                    .ok_or(CatalogError::Corrupt)
            };
        let source_root = root(source_conversation)?;
        if metadata.first_window_id != source_root
            || metadata.previous_window_id != Some(source_previous.unwrap_or(source_root))
        {
            return Err(CatalogError::Corrupt);
        }
        let child_root = root(child)?;
        metadata.conversation_id = child;
        metadata.first_window_id = child_root;
        metadata.window_id = current;
        metadata.previous_window_id = Some(previous.unwrap_or(child_root));
        let research_agent_core::model::ModelMessage::Developer(body) = metadata.into_message()
        else {
            return Err(CatalogError::Corrupt);
        };
        *text = body;
    }
    Ok(())
}

pub(super) fn read_previous(
    db: &Connection,
    parent: Uuid,
    id: Uuid,
) -> Result<ContextCheckpoint, CatalogError> {
    let row: Option<(String, u32, String, u64, Option<String>)> = db.query_row(
        "select checkpoint_json,schema_version,turn_id,source_watermark,previous_window_id from agent_context_checkpoints where conversation_id=?1 and id=?2",
        params![parent.to_string(), id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).optional().map_err(|_| CatalogError::Unavailable)?;
    let (encoded, schema, turn, watermark, previous) = row.ok_or(CatalogError::Corrupt)?;
    if encoded.len() > MAX_CHECKPOINT_BYTES {
        return Err(CatalogError::TooLarge);
    }
    let checkpoint: ContextCheckpoint =
        serde_json::from_str(&encoded).map_err(|_| CatalogError::Corrupt)?;
    checkpoint
        .validate(MAX_CHECKPOINT_BYTES)
        .map_err(|_| CatalogError::Corrupt)?;
    if checkpoint.conversation_id != parent
        || checkpoint.window_id != id
        || checkpoint.schema_version != schema
        || checkpoint.turn_id.to_string() != turn
        || checkpoint.source_watermark != watermark
        || checkpoint.previous_window_id.map(|id| id.to_string()) != previous
    {
        return Err(CatalogError::Corrupt);
    }
    Ok(checkpoint)
}

fn map_error(error: Error) -> CatalogError {
    match error {
        Error::Storage => CatalogError::Unavailable,
        _ => CatalogError::Corrupt,
    }
}
