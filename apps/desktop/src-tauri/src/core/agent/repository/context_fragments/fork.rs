//! Clone trusted contributors in the existing fork publication transaction.
//! Derived from pinned Codex rollout fork/reconstruction (e363b08c9175ac1cbe5893615dd2cb9ddf95043b),
//! Apache-2.0, Copyright 2025 OpenAI. Window UUIDs remain child-owned.
use super::*;
use research_agent_core::conversation::CatalogError;
use std::collections::HashMap;

pub(crate) fn inherit_for_fork(
    db: &Connection,
    parent: Uuid,
    child: Uuid,
    end: u64,
    mut windows: HashMap<Uuid, Uuid>,
) -> Result<(), CatalogError> {
    let supported: bool = db.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_context_fragments')",
        [], |row| row.get(0),
    ).map_err(|_| CatalogError::Unavailable)?;
    if !supported {
        return Ok(());
    }
    let root: Option<String> = db
        .query_row(
            "select window_id from agent_initial_context_windows where conversation_id=?1",
            [parent.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| CatalogError::Unavailable)?;
    let child_root = Uuid::now_v7();
    db.execute(
        "insert into agent_initial_context_windows(conversation_id,window_id) values(?1,?2)",
        params![child.to_string(), child_root.to_string()],
    )
    .map_err(|_| CatalogError::Unavailable)?;
    if let Some(root) = root {
        let root = Uuid::parse_str(&root).map_err(|_| CatalogError::Corrupt)?;
        if root.is_nil() || root.get_version_num() != 7 || windows.contains_key(&root) {
            return Err(CatalogError::Corrupt);
        }
        windows.insert(root, child_root);
        context_windows::initial_metadata::inherit_for_fork(
            db, parent, root, child, child_root, end,
        )
        .map_err(map_error)?;
    }
    let history = context_windows::read_projection(db, child).map_err(map_error)?;
    let mut bytes = 0usize;
    for (source, target) in windows {
        for mut batch in read(db, parent, source, 0).map_err(map_error)? {
            if batch.boundary_seq > end || history.turn(batch.turn_id).is_none() {
                continue;
            }
            batch.conversation_id = child;
            batch.window_id = target;
            if !anchor_valid(&history, &batch) {
                return Err(CatalogError::Corrupt);
            }
            let encoded =
                serde_json::to_string(&batch.fragments).map_err(|_| CatalogError::Corrupt)?;
            bytes = bytes
                .checked_add(encoded.len())
                .ok_or(CatalogError::TooLarge)?;
            if bytes > context_windows::MAX_CHECKPOINT_BYTES {
                return Err(CatalogError::TooLarge);
            }
            db.execute(
                "insert into agent_context_fragments(conversation_id,window_id,boundary_seq,turn_id,anchor_item_id,fragments_json) values(?1,?2,?3,?4,?5,?6)",
                params![child.to_string(), target.to_string(), batch.boundary_seq, batch.turn_id.to_string(), batch.anchor_item_id.to_string(), encoded],
            ).map_err(|_| CatalogError::Unavailable)?;
        }
    }
    Ok(())
}

fn map_error(error: Error) -> CatalogError {
    match error {
        Error::Storage => CatalogError::Unavailable,
        _ => CatalogError::Corrupt,
    }
}
