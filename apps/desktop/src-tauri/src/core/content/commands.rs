use tauri::State;

use super::{
    error::{ContentCommandError, ContentError},
    gc::{run_gc, run_scan},
    gc_manager::ContentGcManager,
    model::{ContentGcRunReport, ContentGcStatus, ContentStorageReport},
    references::storage_report,
};
use crate::storage::AppStore;

#[tauri::command]
pub fn get_content_storage_report(
    store: State<'_, AppStore>,
    manager: State<'_, ContentGcManager>,
) -> Result<ContentStorageReport, ContentCommandError> {
    let (available_bytes, total_disk_bytes) = store.content_disk_space()?;
    let connection = store.connection().map_err(ContentError::from)?;
    storage_report(
        &connection,
        manager.inner(),
        available_bytes,
        total_disk_bytes,
    )
    .map_err(Into::into)
}

#[tauri::command]
pub fn get_content_gc_status(manager: State<'_, ContentGcManager>) -> ContentGcStatus {
    manager.status()
}

#[tauri::command]
pub fn cancel_content_gc(manager: State<'_, ContentGcManager>) -> bool {
    manager.cancel()
}

#[tauri::command]
pub async fn scan_content_storage(
    store: State<'_, AppStore>,
    manager: State<'_, ContentGcManager>,
) -> Result<usize, ContentCommandError> {
    let store = store.inner().clone();
    let manager = manager.inner().clone();
    manager.begin()?;
    let task_manager = manager.clone();
    let task = tauri::async_runtime::spawn_blocking(move || run_scan(&store, &task_manager)).await;
    let result = match task {
        Ok(result) => result,
        Err(_) => Err(ContentError::Cancelled),
    };
    manager.finish();
    result.map_err(Into::into)
}

#[tauri::command]
pub async fn start_content_gc(
    store: State<'_, AppStore>,
    manager: State<'_, ContentGcManager>,
    include_grace_period: bool,
) -> Result<ContentGcRunReport, ContentCommandError> {
    let store = store.inner().clone();
    let manager = manager.inner().clone();
    manager.begin()?;
    let task_manager = manager.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        run_gc(&store, &task_manager, include_grace_period)
    })
    .await;
    let result = match task {
        Ok(result) => result,
        Err(_) => Err(ContentError::Cancelled),
    };
    manager.finish();
    result.map_err(Into::into)
}
