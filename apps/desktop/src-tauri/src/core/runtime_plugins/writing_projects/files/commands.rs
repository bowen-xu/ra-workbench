//! Original file IPC functions; their command names and owned runner stay unchanged.
use super::*;

#[tauri::command]
pub(crate) async fn save_runtime_manuscript_file(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    request: SaveFileRequest,
) -> NativeResult<ManuscriptFileContent> {
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        request.manuscript_id.clone(),
        FileRequest::Save { request },
    )
    .await?
    {
        FileResult::Read(file) => Ok(file),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作文件保存结果种类不匹配。",
            &package_key,
        )),
    }
}
#[tauri::command]
pub(crate) async fn list_runtime_manuscript_files(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Vec<ManuscriptFile>> {
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::List,
    )
    .await?
    {
        FileResult::List(files) => Ok(files),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作文件列表结果种类不匹配。",
            &package_key,
        )),
    }
}

#[tauri::command]
pub(crate) async fn read_runtime_manuscript_file(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    relative_path: String,
) -> NativeResult<ManuscriptFileContent> {
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Read { relative_path },
    )
    .await?
    {
        FileResult::Read(file) => Ok(file),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作文件读取结果种类不匹配。",
            &package_key,
        )),
    }
}
