use super::*;
use research_workbench_native_sdk::writing_project::ManuscriptExportResult;

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Original flat asset payload plus Core states and package binding.
pub(crate) async fn import_runtime_writing_note_asset(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    display_name: String,
    mime_type: String,
    bytes: Vec<u8>,
) -> NativeResult<NoteAsset> {
    match operate_files(
        store.inner().clone(),
        service.inner().clone(),
        catalog.inner().clone(),
        package_key.clone(),
        manuscript_id,
        FileRequest::NoteAsset {
            call: Call::Import {
                display_name,
                mime_type,
                bytes,
            },
        },
    )
    .await?
    {
        FileResult::NoteAsset(asset) => Ok(asset),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "附件导入结果种类不匹配。",
            &package_key,
        )),
    }
}

#[tauri::command]
pub(crate) async fn read_runtime_writing_note_asset(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    asset_id: String,
) -> NativeResult<Vec<u8>> {
    match operate_files(
        store.inner().clone(),
        service.inner().clone(),
        catalog.inner().clone(),
        package_key.clone(),
        manuscript_id,
        FileRequest::NoteAsset {
            call: Call::Read { asset_id },
        },
    )
    .await?
    {
        FileResult::AssetBytes(bytes) => Ok(bytes),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "附件读取结果种类不匹配。",
            &package_key,
        )),
    }
}

#[tauri::command]
pub(crate) async fn export_runtime_writing_note_asset(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    asset_id: String,
    destination_path: String,
) -> NativeResult<ManuscriptExportResult> {
    match operate_files(
        store.inner().clone(),
        service.inner().clone(),
        catalog.inner().clone(),
        package_key.clone(),
        manuscript_id,
        FileRequest::NoteAsset {
            call: Call::Export {
                asset_id,
                destination_path,
            },
        },
    )
    .await?
    {
        FileResult::Exported(result) => Ok(result),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "附件导出结果种类不匹配。",
            &package_key,
        )),
    }
}
