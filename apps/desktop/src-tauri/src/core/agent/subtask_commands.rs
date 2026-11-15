//! Read-only child-task projection for the app's task panel.
//!
//! The conversation history remains authoritative for content. This command
//! exposes only bounded scheduling metadata and never starts or resumes work.

use crate::storage::AppStore;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildTaskSummary {
    pub schema_version: u8,
    pub child_conversation_id: Uuid,
    pub parent_conversation_id: Uuid,
    pub parent_context_snapshot_id: Uuid,
    pub child_context_snapshot_id: Uuid,
    pub depth: u8,
    pub model_calls: u32,
    pub tool_calls: u32,
    pub status: &'static str,
    pub created_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    InvalidInput,
    Conflict,
    StorageUnavailable,
    Corrupt,
}

fn status(value: super::repository::subtasks::Status) -> &'static str {
    match value {
        super::repository::subtasks::Status::Running => "running",
        super::repository::subtasks::Status::Completed => "completed",
        super::repository::subtasks::Status::Canceled => "canceled",
        super::repository::subtasks::Status::Failed => "failed",
        super::repository::subtasks::Status::RecoveryRequired => "recovery_required",
    }
}

fn project(record: super::repository::subtasks::Record) -> ChildTaskSummary {
    ChildTaskSummary {
        schema_version: 1,
        child_conversation_id: record.child_conversation_id,
        parent_conversation_id: record.parent_conversation_id,
        parent_context_snapshot_id: record.parent_context_snapshot_id,
        child_context_snapshot_id: record.child_context_snapshot_id,
        depth: record.depth,
        model_calls: record.model_calls,
        tool_calls: record.tool_calls,
        status: status(record.status),
        created_at: record.created_at,
        finished_at: record.finished_at,
    }
}

#[tauri::command]
#[allow(private_interfaces)]
pub fn agent_list_child_tasks(
    store: State<'_, AppStore>,
    parent_conversation_id: Uuid,
    limit: Option<u16>,
) -> Result<Vec<ChildTaskSummary>, Error> {
    list(&store, parent_conversation_id, limit)
}

pub(crate) fn list(
    store: &AppStore,
    parent_conversation_id: Uuid,
    limit: Option<u16>,
) -> Result<Vec<ChildTaskSummary>, Error> {
    let limit = limit.unwrap_or(64);
    if parent_conversation_id.is_nil() || limit == 0 || limit > 128 {
        return Err(Error::InvalidInput);
    }
    let db = store.connection().map_err(|_| Error::StorageUnavailable)?;
    super::repository::subtasks::list_for_parent(&db, parent_conversation_id, limit)
        .map_err(|error| match error {
            super::repository::subtasks::Error::Storage => Error::StorageUnavailable,
            super::repository::subtasks::Error::Invalid => Error::Corrupt,
        })
        .map(|records| records.into_iter().map(project).collect())
}

#[tauri::command]
#[allow(private_interfaces)]
pub fn agent_reconcile_child_task(
    store: State<'_, AppStore>,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
) -> Result<ChildTaskSummary, Error> {
    reconcile(&store, parent_conversation_id, child_conversation_id)
}

pub(crate) fn reconcile(
    store: &AppStore,
    parent_conversation_id: Uuid,
    child_conversation_id: Uuid,
) -> Result<ChildTaskSummary, Error> {
    if parent_conversation_id.is_nil() || child_conversation_id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let mut db = store.connection().map_err(|_| Error::StorageUnavailable)?;
    let record = super::repository::subtasks::find_by_child(&db, child_conversation_id)
        .map_err(|error| match error {
            super::repository::subtasks::Error::Storage => Error::StorageUnavailable,
            super::repository::subtasks::Error::Invalid => Error::Corrupt,
        })?
        .ok_or(Error::Conflict)?;
    if record.parent_conversation_id != parent_conversation_id {
        return Err(Error::Conflict);
    }
    let changed = super::repository::subtasks::reconcile_terminal(&mut db, child_conversation_id)
        .map_err(|error| match error {
        super::repository::subtasks::Error::Storage => Error::StorageUnavailable,
        super::repository::subtasks::Error::Invalid => Error::Corrupt,
    })?;
    let updated =
        super::repository::subtasks::find_by_child(&db, child_conversation_id).map_err(|error| {
            match error {
                super::repository::subtasks::Error::Storage => Error::StorageUnavailable,
                super::repository::subtasks::Error::Invalid => Error::Corrupt,
            }
        });
    drop(db);
    if changed {
        store.notify_agent_subtasks_changed(Some(parent_conversation_id));
    }
    updated?.map(project).ok_or(Error::Conflict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_stable_for_projection() {
        assert_eq!(
            status(super::super::repository::subtasks::Status::RecoveryRequired),
            "recovery_required"
        );
    }
}
