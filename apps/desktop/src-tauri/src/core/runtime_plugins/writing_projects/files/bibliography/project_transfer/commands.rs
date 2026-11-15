use super::*;

pub(crate) async fn run_import(
    store: AppStore,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    package_key: String,
    call: Call,
) -> NativeResult<ManuscriptTransferResult> {
    let id = uuid::Uuid::new_v4().to_string();
    match super::super::super::operate_files(
        store,
        service,
        catalog,
        package_key.clone(),
        id,
        FileRequest::Bibliography { call },
    )
    .await?
    {
        FileResult::ProjectImported(result) => Ok(result),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "工程导入结果种类不匹配。",
            &package_key,
        )),
    }
}

#[tauri::command]
pub(crate) async fn import_runtime_manuscript_project(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    title: String,
    source_path: String,
    root_path: Option<String>,
) -> NativeResult<ManuscriptTransferResult> {
    run_import(
        store.inner().clone(),
        service.inner().clone(),
        catalog.inner().clone(),
        package_key,
        Call::ImportProject {
            title,
            source_path: source_path.trim().into(),
            requested_root: root_path,
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn duplicate_runtime_manuscript(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Manuscript> {
    run_import(
        store.inner().clone(),
        service.inner().clone(),
        catalog.inner().clone(),
        package_key,
        Call::DuplicateProject {
            source_id: manuscript_id,
        },
    )
    .await
    .map(|result| result.manuscript)
}
