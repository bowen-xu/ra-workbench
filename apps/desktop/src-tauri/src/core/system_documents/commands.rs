use super::*;
use crate::storage::AppStore;
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Write {
    pub id: String,
    pub expected_revision: String,
    pub content: String,
    pub baseline: String,
}
#[tauri::command]
pub async fn system_documents_list(store: State<'_, AppStore>) -> Result<Vec<Document>, String> {
    let root = store.agent_configuration_directory().to_path_buf();
    tokio::task::spawn_blocking(move || storage::list(&root))
        .await
        .map_err(|_| "无法读取内置文档")?
}
#[tauri::command]
pub async fn system_documents_read(
    store: State<'_, AppStore>,
    id: String,
) -> Result<Document, String> {
    let root = store.agent_configuration_directory().to_path_buf();
    tokio::task::spawn_blocking(move || read(&root, &id))
        .await
        .map_err(|_| "无法读取内置文档")?
}
/// This IPC is a human editor/review action, never a registered model tool.
#[tauri::command]
pub async fn system_documents_write(
    store: State<'_, AppStore>,
    input: Write,
) -> Result<Document, String> {
    let root = store.agent_configuration_directory().to_path_buf();
    tokio::task::spawn_blocking(move || {
        write(
            &root,
            &input.id,
            &input.expected_revision,
            &input.content,
            &input.baseline,
        )
    })
    .await
    .map_err(|_| "无法保存内置文档")?
}
#[tauri::command]
pub async fn system_documents_pending(
    store: State<'_, AppStore>,
    conversation_id: Uuid,
) -> Result<Vec<approvals::Request>, String> {
    store.system_document_approvals().pending(conversation_id)
}
#[tauri::command]
pub async fn system_documents_decide(
    store: State<'_, AppStore>,
    conversation_id: Uuid,
    request_id: Uuid,
    allow: bool,
) -> Result<(), String> {
    store
        .system_document_approvals()
        .decide(conversation_id, request_id, allow)
}

/// Reveal the actual backing object, not the logical source:// address.
#[tauri::command]
pub async fn system_documents_location(
    store: State<'_, AppStore>,
    id: String,
) -> Result<String, String> {
    let root = store.agent_configuration_directory().to_path_buf();
    tokio::task::spawn_blocking(move || {
        registry::definition(&id)?;
        let root = root.canonicalize().map_err(|_| "找不到本地文档存储")?;
        let backing = root.join("builtin-documents.v1.json");
        let target = if backing.is_file() { backing } else { root };
        Ok(target.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "无法定位内置文档")?
}

/// Export editor text to the path selected by the human in the save dialog.
#[tauri::command]
pub async fn text_document_export(destination: String, content: String) -> Result<(), String> {
    let destination = std::path::PathBuf::from(destination);
    if !destination.is_absolute() || content.len() > 16 * 1024 * 1024 {
        return Err("另存为路径或文本大小无效".into());
    }
    tokio::task::spawn_blocking(move || {
        crate::export_file::write_new_file(&destination, content.as_bytes()).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "目标文件已存在，请选择新的文件名。".to_owned()
            } else {
                "无法另存为文本文件。".to_owned()
            }
        })
    })
    .await
    .map_err(|_| "无法另存为文本文件")?
}
