use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::storage::AppStore;

use super::{
    error::{NotesCommandError, NotesError},
    transfer_manager::NoteTransferManager,
};

const MAX_EXPORT_FILES: usize = 2_000;
const MAX_EXPORT_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoteExportFile {
    relative_path: String,
    contents: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoteExportAsset {
    id: String,
    relative_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteExportRequest {
    target_directory: String,
    folder_name: String,
    manifest_json: String,
    files: Vec<NoteExportFile>,
    assets: Vec<NoteExportAsset>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteExportReport {
    folder_name: String,
    file_count: usize,
    asset_count: usize,
}

fn safe_relative_path(value: &str) -> Result<PathBuf, NotesError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(NotesError::InvalidInput("导出相对路径无效".to_owned()));
    }
    Ok(path.to_path_buf())
}

fn validate_request(request: &NoteExportRequest) -> Result<(), NotesError> {
    safe_relative_path(&request.folder_name)?;
    if Path::new(&request.folder_name).components().count() != 1 {
        return Err(NotesError::InvalidInput("导出目录名称无效".to_owned()));
    }
    if request.files.len() + request.assets.len() > MAX_EXPORT_FILES
        || request.manifest_json.len()
            + request
                .files
                .iter()
                .map(|file| file.contents.len())
                .sum::<usize>()
            > MAX_EXPORT_TEXT_BYTES
    {
        return Err(NotesError::InvalidInput(
            "导出内容超过 P0 安全上限".to_owned(),
        ));
    }
    serde_json::from_str::<serde_json::Value>(&request.manifest_json)
        .map_err(|_| NotesError::InvalidInput("导出 manifest 无效".to_owned()))?;
    for file in &request.files {
        safe_relative_path(&file.relative_path)?;
    }
    for asset in &request.assets {
        safe_relative_path(&asset.relative_path)?;
        if asset.id.len() != 64 || !asset.id.bytes().all(|value| value.is_ascii_hexdigit()) {
            return Err(NotesError::InvalidInput("导出附件 ID 无效".to_owned()));
        }
    }
    Ok(())
}

fn write_export(
    store: &AppStore,
    manager: &NoteTransferManager,
    request: &NoteExportRequest,
) -> Result<NoteExportReport, NotesError> {
    validate_request(request)?;
    let target = fs::canonicalize(&request.target_directory)
        .map_err(|_| NotesError::InvalidInput("所选导出目录不可用".to_owned()))?;
    if !target.is_dir() {
        return Err(NotesError::InvalidInput("所选导出目标不是目录".to_owned()));
    }
    let destination = target.join(&request.folder_name);
    if destination.exists() {
        return Err(NotesError::InvalidInput(
            "同名导出目录已存在，不会覆盖".to_owned(),
        ));
    }
    let temporary = target.join(format!(".note-export-{}.tmp", Uuid::new_v4()));
    fs::create_dir(&temporary)?;
    let result = (|| -> Result<(), NotesError> {
        manager.check_cancelled()?;
        fs::write(temporary.join("manifest.json"), &request.manifest_json)?;
        for file in &request.files {
            manager.check_cancelled()?;
            let output = temporary.join(safe_relative_path(&file.relative_path)?);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(output, &file.contents)?;
        }
        let connection = store.connection()?;
        for asset in &request.assets {
            manager.check_cancelled()?;
            let stored_path = connection
                .query_row(
                    "select relative_path from content_objects where sha256 = ?1",
                    [&asset.id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .ok_or(NotesError::NotFound)?;
            let output = temporary.join(safe_relative_path(&asset.relative_path)?);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(store.content_directory().join(stored_path), output)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    fs::rename(&temporary, &destination)?;
    Ok(NoteExportReport {
        folder_name: request.folder_name.clone(),
        file_count: request.files.len() + 1,
        asset_count: request.assets.len(),
    })
}

#[tauri::command]
pub async fn export_notes(
    store: State<'_, AppStore>,
    manager: State<'_, NoteTransferManager>,
    request: NoteExportRequest,
) -> Result<NoteExportReport, NotesCommandError> {
    let store = store.inner().clone();
    let manager = manager.inner().clone();
    manager.begin()?;
    let worker_manager = manager.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        write_export(&store, &worker_manager, &request)
    })
    .await;
    manager.finish();
    result
        .map_err(|error| NotesError::TransferFailed(error.to_string()))?
        .map_err(Into::into)
}

#[tauri::command]
pub fn cancel_note_export(manager: State<'_, NoteTransferManager>) -> bool {
    manager.cancel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_export_path_traversal() {
        assert!(safe_relative_path("../private.md").is_err());
        assert!(safe_relative_path("/tmp/private.md").is_err());
        assert!(safe_relative_path("pages/note.md").is_ok());
    }
}
