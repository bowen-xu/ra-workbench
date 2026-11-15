use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::storage::AppStore;

use super::error::{NotesCommandError, NotesError};
use super::transfer_manager::NoteTransferManager;

const MAX_NOTE_ASSET_BYTES: usize = 16 * 1024 * 1024;
const ASSET_IO_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub(crate) struct NoteAssetManager(NoteTransferManager);

impl NoteAssetManager {
    fn begin(&self) -> Result<(), NotesError> {
        self.0.begin()
    }

    fn finish(&self) {
        self.0.finish();
    }

    fn cancel(&self) -> bool {
        self.0.cancel()
    }

    fn check_cancelled(&self) -> Result<(), NotesError> {
        self.0.check_cancelled()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteAssetImportRequest {
    pub(crate) display_name: String,
    pub(crate) mime_type: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteAsset {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) mime_type: String,
    byte_size: usize,
}

fn validate_asset_request(request: &NoteAssetImportRequest) -> Result<(), NotesError> {
    if request.bytes.is_empty() || request.bytes.len() > MAX_NOTE_ASSET_BYTES {
        return Err(NotesError::InvalidInput(
            "笔记附件必须介于 1 字节和 16 MB 之间".to_owned(),
        ));
    }
    if request.display_name.trim().is_empty() || request.display_name.chars().count() > 240 {
        return Err(NotesError::InvalidInput("附件名称无效".to_owned()));
    }
    if request.mime_type.trim().is_empty()
        || request.mime_type.len() > 160
        || !request.mime_type.is_ascii()
    {
        return Err(NotesError::InvalidInput("附件 MIME 类型无效".to_owned()));
    }
    Ok(())
}

fn hash_asset_bytes(manager: &NoteAssetManager, bytes: &[u8]) -> Result<String, NotesError> {
    let mut hasher = Sha256::new();
    for chunk in bytes.chunks(ASSET_IO_CHUNK_BYTES) {
        manager.check_cancelled()?;
        hasher.update(chunk);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn import_note_asset_from(
    store: &AppStore,
    manager: &NoteAssetManager,
    request: NoteAssetImportRequest,
) -> Result<NoteAsset, NotesError> {
    let _content_read = store.content_read()?;
    store.ensure_content_import_capacity(request.bytes.len() as u64)?;
    let mut connection = store.connection()?;
    let asset = import_note_asset_at(&mut connection, store.content_directory(), manager, request)?;
    store.retain_content_for_session(&asset.id)?;
    Ok(asset)
}

/// Shared importer for an explicitly supplied content store. The caller owns
/// capacity/lifecycle admission; validation, hashing and persistence stay here.
pub(crate) fn import_note_asset_at(
    connection: &mut rusqlite::Connection,
    directory: &Path,
    manager: &NoteAssetManager,
    request: NoteAssetImportRequest,
) -> Result<NoteAsset, NotesError> {
    validate_asset_request(&request)?;
    let id = hash_asset_bytes(manager, &request.bytes)?;
    let relative_path = format!("{}/{}/{}", &id[..2], &id[2..4], id);
    let destination = directory.join(&relative_path);
    let mut temporary = None;
    if !destination.exists() {
        let parent = destination
            .parent()
            .ok_or_else(|| NotesError::InvalidInput("附件存储路径无效".to_owned()))?;
        fs::create_dir_all(parent)?;
        let path = parent.join(format!(".note-asset-{}.tmp", Uuid::new_v4()));
        let write_result = (|| -> Result<(), NotesError> {
            let mut file = File::create(&path)?;
            for chunk in request.bytes.chunks(ASSET_IO_CHUNK_BYTES) {
                manager.check_cancelled()?;
                file.write_all(chunk)?;
            }
            file.flush()?;
            manager.check_cancelled()
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        temporary = Some(path);
    }

    let persistence_result = (|| -> Result<(), NotesError> {
        let transaction = connection.transaction()?;
        transaction.execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(sha256) do nothing",
            params![
                id,
                relative_path,
                request.mime_type,
                request.bytes.len() as i64
            ],
        )?;
        manager.check_cancelled()?;
        if let Some(path) = temporary.as_ref() {
            if let Err(error) = fs::rename(path, &destination) {
                if !destination.exists() {
                    return Err(error.into());
                }
            } else {
                temporary = None;
            }
        }
        transaction.commit()?;
        Ok(())
    })();
    if let Err(error) = persistence_result {
        if let Some(path) = temporary.as_ref() {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    if let Some(path) = temporary.as_ref() {
        let _ = fs::remove_file(path);
    }
    Ok(NoteAsset {
        display_name: crate::content::note_image_display_name(&id, &request.mime_type)
            .unwrap_or_else(|| request.display_name.trim().to_owned()),
        id,
        mime_type: request.mime_type,
        byte_size: request.bytes.len(),
    })
}

#[tauri::command]
pub async fn import_note_asset(
    store: State<'_, AppStore>,
    manager: State<'_, NoteAssetManager>,
    request: NoteAssetImportRequest,
) -> Result<NoteAsset, NotesCommandError> {
    let store = store.inner().clone();
    let manager = manager.inner().clone();
    manager.begin()?;
    let task_manager = manager.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        import_note_asset_from(&store, &task_manager, request)
    })
    .await
    .map_err(|error| NotesError::TransferFailed(error.to_string()));
    manager.finish();
    result?.map_err(Into::into)
}

#[tauri::command]
pub fn cancel_note_asset_import(manager: State<'_, NoteAssetManager>) -> bool {
    manager.cancel()
}

fn read_note_asset_from(store: &AppStore, id: String) -> Result<Vec<u8>, NotesError> {
    let _content_read = store.content_read()?;
    let connection = store.connection()?;
    let bytes = read_note_asset_at(&connection, store.content_directory(), &id)?;
    store.retain_content_for_session(&id)?;
    Ok(bytes)
}

pub(crate) fn read_note_asset_at(
    connection: &rusqlite::Connection,
    directory: &std::path::Path,
    id: &str,
) -> Result<Vec<u8>, NotesError> {
    if id.len() != 64 || !id.bytes().all(|value| value.is_ascii_hexdigit()) {
        return Err(NotesError::InvalidInput("附件 ID 无效".to_owned()));
    }
    let relative_path = {
        connection
            .query_row(
                "select relative_path from content_objects where sha256 = ?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or(NotesError::NotFound)?
    };
    let file = directory.join(relative_path).canonicalize()?;
    if !file.starts_with(directory.canonicalize()?) {
        return Err(NotesError::InvalidInput("附件路径不属于对象存储".into()));
    }
    fs::read(file).map_err(Into::into)
}

#[tauri::command]
pub async fn read_note_asset(
    store: State<'_, AppStore>,
    id: String,
) -> Result<Vec<u8>, NotesCommandError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || read_note_asset_from(&store, id))
        .await
        .map_err(|error| NotesError::TransferFailed(error.to_string()))?
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn asset_store() -> (AppStore, std::path::PathBuf) {
        let content_directory =
            std::env::temp_dir().join(format!("research-assistant-note-asset-{}", Uuid::new_v4()));
        fs::create_dir_all(&content_directory).expect("create content directory");
        let connection = Connection::open_in_memory().expect("open database");
        connection
            .execute_batch(
                "create table content_objects (
                sha256 text primary key,
                relative_path text not null,
                mime_type text not null,
                byte_size integer not null,
                created_at text not null
            );",
            )
            .expect("create content table");
        (
            AppStore::for_test(connection, content_directory.clone()),
            content_directory,
        )
    }

    #[test]
    fn imports_asset_atomically_and_registers_content_object() {
        let (store, content_directory) = asset_store();
        let manager = NoteAssetManager::default();
        manager.begin().expect("start asset import");
        let asset = import_note_asset_from(
            &store,
            &manager,
            NoteAssetImportRequest {
                display_name: "evidence.txt".to_owned(),
                mime_type: "text/plain".to_owned(),
                bytes: b"replicable evidence".to_vec(),
            },
        )
        .expect("import asset");
        manager.finish();
        assert_eq!(asset.display_name, "evidence.txt");

        let relative_path = format!("{}/{}/{}", &asset.id[..2], &asset.id[2..4], asset.id);
        assert_eq!(
            fs::read(content_directory.join(&relative_path)).expect("read asset"),
            b"replicable evidence"
        );
        let connection = store.connection().expect("database");
        let registered_path: String = connection
            .query_row(
                "select relative_path from content_objects where sha256 = ?1",
                [&asset.id],
                |row| row.get(0),
            )
            .expect("registered content object");
        assert_eq!(registered_path, relative_path);
        drop(connection);
        fs::remove_dir_all(content_directory).expect("remove content directory");
    }

    #[test]
    fn cancelled_asset_import_stops_before_writing_content() {
        let (store, content_directory) = asset_store();
        let manager = NoteAssetManager::default();
        manager.begin().expect("start asset import");
        assert!(manager.cancel());
        let request = NoteAssetImportRequest {
            display_name: "cancelled.bin".to_owned(),
            mime_type: "application/octet-stream".to_owned(),
            bytes: vec![1; ASSET_IO_CHUNK_BYTES],
        };
        let result = import_note_asset_from(&store, &manager, request);
        assert!(matches!(result, Err(NotesError::OperationCancelled)));
        assert_eq!(
            fs::read_dir(&content_directory)
                .expect("read content directory")
                .count(),
            0
        );
        manager.finish();
        fs::remove_dir_all(content_directory).expect("remove content directory");
    }

    #[test]
    fn image_metadata_uses_content_identity_without_changing_hash_or_storage_path() {
        let (store, directory) = asset_store();
        let manager = NoteAssetManager::default();
        let import = |name: &str| {
            import_note_asset_from(
                &store,
                &manager,
                NoteAssetImportRequest {
                    display_name: name.into(),
                    mime_type: "image/png".into(),
                    bytes: b"synthetic image bytes".to_vec(),
                },
            )
            .expect("import image")
        };
        let first = import("original-screenshot.png");
        let second = import("renamed.png");
        assert!(
            store
                .is_content_session_retained(&first.id)
                .expect("session lease")
        );
        assert_eq!(first.id, second.id);
        assert_eq!(first.display_name, format!("{}.png", first.id));
        assert_eq!(second.display_name, first.display_name);
        let relative = format!("{}/{}/{}", &first.id[..2], &first.id[2..4], first.id);
        assert_eq!(
            fs::read(directory.join(relative)).expect("read image"),
            b"synthetic image bytes"
        );
        drop(store);
        fs::remove_dir_all(directory).expect("remove content directory");
    }

    #[test]
    fn reading_an_existing_asset_retains_it_for_session_undo() {
        let (store, directory) = asset_store();
        let asset = import_note_asset_at(
            &mut store.connection().expect("database"),
            &directory,
            &NoteAssetManager::default(),
            NoteAssetImportRequest {
                display_name: "old-image.png".into(),
                mime_type: "image/png".into(),
                bytes: b"existing bytes".to_vec(),
            },
        )
        .expect("existing stored asset");
        assert!(
            !store
                .is_content_session_retained(&asset.id)
                .expect("no session lease yet")
        );
        assert_eq!(
            read_note_asset_from(&store, asset.id.clone()).expect("read asset"),
            b"existing bytes"
        );
        assert!(
            store
                .is_content_session_retained(&asset.id)
                .expect("session lease")
        );
        drop(store);
        fs::remove_dir_all(directory).expect("remove content directory");
    }
}

#[cfg(test)]
#[path = "assets/hbs_tests.rs"]
mod hbs_tests;
