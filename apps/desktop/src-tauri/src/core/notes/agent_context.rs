//! Frozen structured note evidence for shared Agent hosts. This is a local read,
//! not a model disclosure grant; references never recursively expand its scope.
use super::{document_repository::load_document_from, document_validation::validate_document};
use crate::storage::AppStore;
use research_agent_core::submission::{ContextSourceKind, ContextText};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteContextRequest {
    pub page_id: Uuid,
    pub expected_revision: i64,
    /// None includes the page; Some includes precisely this block and descendants.
    pub block_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum NoteContextError {
    #[error("invalid note context selection")]
    InvalidInput,
    #[error("note context changed; refresh its snapshot")]
    Conflict,
    #[error("note context unavailable or invalid")]
    Unavailable,
    #[error("note context exceeds its budget")]
    Budget,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContextPreview {
    pub schema_version: u8,
    pub source: ContextText,
}

/// Local disclosure preview only. No conversation, model request or permission
/// record is created; native submission preparation must recheck scope/revision.
#[tauri::command]
pub async fn agent_preview_note_context(
    store: tauri::State<'_, AppStore>,
    request: NoteContextRequest,
) -> Result<NoteContextPreview, NoteContextError> {
    Ok(NoteContextPreview {
        schema_version: 1,
        source: materialize(&store, request).await?,
    })
}

/// Freeze the saved revision in one database snapshot on a blocking worker.
/// Dropping a waiter neither modifies a note nor sends its contents to a model.
/// The caller must separately authorize the selected resource and disclosure.
pub async fn materialize(
    store: &AppStore,
    request: NoteContextRequest,
) -> Result<ContextText, NoteContextError> {
    validate_request(&request)?;
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut connection = store
            .connection()
            .map_err(|_| NoteContextError::Unavailable)?;
        let tx = connection
            .transaction()
            .map_err(|_| NoteContextError::Unavailable)?;
        materialize_in(&tx, &request)
    })
    .await
    .map_err(|_| NoteContextError::Unavailable)?
}

pub(crate) fn validate_request(request: &NoteContextRequest) -> Result<(), NoteContextError> {
    if request.page_id.is_nil()
        || request.expected_revision < 0
        || request
            .block_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.len() > 128 || id.contains('\0'))
    {
        return Err(NoteContextError::InvalidInput);
    }
    Ok(())
}

/// Caller owns a transaction spanning source checks and any snapshot insertion.
pub(crate) fn materialize_in(
    connection: &Connection,
    request: &NoteContextRequest,
) -> Result<ContextText, NoteContextError> {
    validate_request(request)?;
    let page_id = request.page_id.to_string();
    // The interactive loader tolerates malformed JSON and orphaned blocks for
    // recovery. Evidence cannot silently substitute empty content or lose nodes.
    let (count, bytes, invalid): (usize, usize, usize) = connection.query_row(
        "select count(*), coalesce(sum(length(cast(props_json as blob)) + length(cast(content_json as blob))),0),
         coalesce(sum(case when schema_version<>1 or json_valid(props_json)=0 or json_valid(content_json)=0
         or (parent_block_id is not null and not exists(select 1 from note_blocks p
             where p.id=b.parent_block_id and p.page_id=b.page_id and p.scope_kind='page' and p.status='active'))
         then 1 else 0 end),0)
         from note_blocks b where page_id=?1 and scope_kind='page' and status='active'",
        params![page_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|_| NoteContextError::Unavailable)?;
    if bytes > MAX_BYTES || count > 20_000 {
        return Err(NoteContextError::Budget);
    }
    if invalid != 0 {
        return Err(NoteContextError::Unavailable);
    }
    // Bound depth before invoking the existing recursive document builder.
    let reachable: usize = connection.query_row(
        "with recursive tree(id,depth) as (
          select id,1 from note_blocks where page_id=?1 and scope_kind='page' and status='active' and parent_block_id is null
          union all select b.id,t.depth+1 from note_blocks b join tree t on b.parent_block_id=t.id
          where b.page_id=?1 and b.scope_kind='page' and b.status='active' and t.depth<64
         ) select count(*) from tree", [&page_id], |row| row.get(0),
    ).map_err(|_| NoteContextError::Unavailable)?;
    if reachable != count {
        return Err(NoteContextError::Unavailable);
    }
    let snapshot =
        load_document_from(connection, &page_id).map_err(|_| NoteContextError::Unavailable)?;
    if snapshot.page.status != "active" {
        return Err(NoteContextError::Unavailable);
    }
    if snapshot.page.revision != request.expected_revision {
        return Err(NoteContextError::Conflict);
    }
    validate_document(&snapshot.document).map_err(|_| NoteContextError::Unavailable)?;
    fn count_blocks(blocks: &[super::model::NoteDocumentBlock]) -> usize {
        blocks
            .iter()
            .map(|block| 1 + count_blocks(&block.children))
            .sum()
    }
    if count_blocks(&snapshot.document.blocks) != count {
        return Err(NoteContextError::Unavailable);
    }
    fn find<'a>(
        blocks: &'a [super::model::NoteDocumentBlock],
        id: &str,
    ) -> Option<&'a super::model::NoteDocumentBlock> {
        blocks.iter().find_map(|block| {
            if block.id == id {
                Some(block)
            } else {
                find(&block.children, id)
            }
        })
    }
    let blocks = match &request.block_id {
        Some(id) => std::slice::from_ref(
            find(&snapshot.document.blocks, id).ok_or(NoteContextError::Conflict)?,
        ),
        None => snapshot.document.blocks.as_slice(),
    };
    let text = serde_json::to_string(&serde_json::json!({
        "schemaVersion": 1, "pageId": page_id, "title": snapshot.page.title,
        "pageRevision": snapshot.page.revision, "blocks": blocks,
    }))
    .map_err(|_| NoteContextError::Unavailable)?;
    if text.len() > MAX_BYTES {
        return Err(NoteContextError::Budget);
    }
    let identity = serde_json::to_vec(&(request.page_id, &request.block_id))
        .map_err(|_| NoteContextError::Unavailable)?;
    Ok(ContextText {
        source_id: format!("note:{:x}", Sha256::digest(identity)),
        kind: ContextSourceKind::Note,
        resource_id: page_id.clone(),
        revision: format!(
            "revision:{}:sha256:{:x}",
            snapshot.page.revision,
            Sha256::digest(text.as_bytes())
        ),
        anchor: Some(
            serde_json::json!({"schemaVersion":1,"pageId":page_id,
            "pageRevision":snapshot.page.revision,"blockId":request.block_id})
            .to_string(),
        ),
        text,
    })
}

#[cfg(test)]
#[path = "agent_context/tests.rs"]
mod tests;
