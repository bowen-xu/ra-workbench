use super::{
    error::{NotesCommandError, NotesError},
    metadata_batch::{self, BatchItem, BatchReceipt, BatchRequest},
    metadata_entry_history::{self, Entries},
    page_repository::page_from,
};
use crate::storage::AppStore;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tauri::State;
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Status {
    pub id: String,
    pub status: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct History {
    pub statuses: Vec<Status>,
    pub entries: Vec<Entries>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayRequest {
    operation_id: String,
    source_operation_id: String,
    direction: String,
    items: Vec<BatchItem>,
}
pub(super) fn capture(
    tx: &Transaction<'_>,
    targets: &HashSet<String>,
    statuses: Vec<(String, String)>,
) -> Result<History, NotesError> {
    Ok(History {
        statuses: statuses
            .into_iter()
            .map(|(id, status)| Status { id, status })
            .collect(),
        entries: metadata_entry_history::capture_sources(tx, targets)?,
    })
}
pub(crate) fn replay_from(
    connection: &mut Connection,
    request: ReplayRequest,
) -> Result<BatchReceipt, NotesError> {
    if !matches!(request.direction.as_str(), "undo" | "redo")
        || request.operation_id.is_empty()
        || request.operation_id.len() > 128
        || request.operation_id == request.source_operation_id
    {
        return Err(NotesError::InvalidInput(
            "note_history_invalid: 无效的撤销请求".into(),
        ));
    }
    let encoded =
        serde_json::to_string(&request).map_err(|e| NotesError::InvalidInput(e.to_string()))?;
    let tx = connection.transaction()?;
    let prior:Option<(String,String)>=tx.query_row("select request_json,receipt_json from note_metadata_batch_receipts where operation_id=?1",[&request.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((identity, receipt)) = prior {
        if identity != encoded {
            return Err(NotesError::InvalidInput("操作 ID 已用于不同请求".into()));
        }
        return serde_json::from_str(&receipt).map_err(|e| NotesError::InvalidInput(e.to_string()));
    }
    let raw: String = tx.query_row(
        "select receipt_json from note_metadata_batch_receipts where operation_id=?1",
        [&request.source_operation_id],
        |r| r.get(0),
    )?;
    let original: BatchReceipt =
        serde_json::from_str(&raw).map_err(|e| NotesError::InvalidInput(e.to_string()))?;
    let history = original.history.ok_or_else(|| {
        NotesError::InvalidInput("note_history_unavailable: 原操作不可撤销".into())
    })?;
    if request.items.len() != history.statuses.len()
        || history
            .statuses
            .iter()
            .any(|old| !request.items.iter().any(|item| item.id == old.id))
    {
        return Err(NotesError::InvalidInput(
            "note_history_changed: 撤销范围已变化".into(),
        ));
    }
    for old in &history.statuses {
        let current = page_from(&tx, &old.id)?;
        if request
            .items
            .iter()
            .find(|item| item.id == old.id)
            .is_some_and(|item| item.expected_revision != current.revision)
        {
            return Err(NotesError::RevisionConflict {
                current_revision: current.revision,
            });
        }
        let expected = if request.direction == "undo" {
            "trashed"
        } else {
            &old.status
        };
        if current.status != expected {
            return Err(NotesError::InvalidInput(
                "note_history_changed: 笔记状态已变化".into(),
            ));
        }
    }
    if request.direction == "redo" {
        let mut query=tx.prepare("select id,parent_page_id from note_pages where status<>'trashed' and parent_page_id is not null")?;
        let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, parent) = row?;
            if history.statuses.iter().any(|v| v.id == parent)
                && !history.statuses.iter().any(|v| v.id == id)
            {
                return Err(NotesError::InvalidInput(
                    "note_history_changed: 笔记新增了子页面".into(),
                ));
            }
        }
    }
    let mut receipt = metadata_batch::apply_in(
        &tx,
        BatchRequest {
            operation_id: request.operation_id.clone(),
            action: if request.direction == "undo" {
                "restore"
            } else {
                "trash"
            }
            .into(),
            items: request.items,
        },
        false,
    )?;
    if !receipt.failures.is_empty() {
        return Err(NotesError::InvalidInput(
            "note_history_changed: 笔记修订已变化".into(),
        ));
    }
    if request.direction == "undo" {
        for old in &history.statuses {
            if old.status != "active" {
                tx.execute(
                    "update note_pages set status=?2 where id=?1",
                    params![old.id, old.status],
                )?;
                receipt
                    .body_changed_ids
                    .extend(super::page_repository::sync_child_page_block(&tx, &old.id)?);
            }
        }
        receipt
            .body_changed_ids
            .extend(metadata_entry_history::restore_sources(
                &tx,
                &history.entries,
            )?);
        receipt.body_changed_ids.sort();
        receipt.body_changed_ids.dedup();
        super::page_entries::synchronize_baselines(
            &tx,
            &receipt.body_changed_ids.iter().cloned().collect(),
            &history.statuses.iter().map(|v| v.id.clone()).collect(),
            &HashSet::new(),
        )?;
        let mut ids: HashSet<_> = receipt.pages.iter().map(|p| p.id.clone()).collect();
        ids.extend(receipt.body_changed_ids.iter().cloned());
        receipt.pages = ids
            .iter()
            .map(|id| page_from(&tx, id))
            .collect::<Result<_, _>>()?;
    }
    let result =
        serde_json::to_string(&receipt).map_err(|e| NotesError::InvalidInput(e.to_string()))?;
    tx.execute("update note_metadata_batch_receipts set request_json=?2,receipt_json=?3 where operation_id=?1",params![request.operation_id,encoded,result])?;
    tx.commit()?;
    Ok(receipt)
}
#[tauri::command]
pub fn replay_note_metadata_batch(
    store: State<'_, AppStore>,
    request: ReplayRequest,
) -> Result<BatchReceipt, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    replay_from(&mut connection, request).map_err(Into::into)
}
#[cfg(test)]
#[path = "metadata_history/tests.rs"]
mod tests;
