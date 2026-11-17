use super::super::{RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage};
use super::ManagedPluginListing;
use std::{path::PathBuf, sync::Arc};

async fn worker<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, RuntimePluginError> + Send + 'static,
) -> Result<T, RuntimePluginError> {
    tokio::task::spawn_blocking(run).await.map_err(|_| {
        RuntimePluginError::new(
            "plugin_management_worker_failed",
            "本地插件管理任务未完成，请刷新核对安装记录。",
        )
    })?
}

#[tauri::command]
pub(crate) async fn list_managed_plugins(
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
) -> Result<ManagedPluginListing, RuntimePluginError> {
    let catalog = catalog.inner().clone();
    worker(move || catalog.managed_plugins()).await
}

#[tauri::command]
pub(crate) async fn install_managed_plugin(
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    source: PathBuf,
    expected_revision: String,
) -> Result<RuntimePluginPackage, RuntimePluginError> {
    let catalog = catalog.inner().clone();
    worker(move || catalog.install_managed_plugin(&source, &expected_revision)).await
}

#[tauri::command]
pub(crate) async fn set_managed_plugin_enabled(
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    plugin_id: String,
    enabled: bool,
    expected_revision: String,
) -> Result<(), RuntimePluginError> {
    let catalog = catalog.inner().clone();
    worker(move || catalog.set_managed_plugin(&plugin_id, enabled, false, &expected_revision)).await
}

#[tauri::command]
pub(crate) async fn uninstall_managed_plugin(
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    plugin_id: String,
    expected_revision: String,
) -> Result<(), RuntimePluginError> {
    let catalog = catalog.inner().clone();
    worker(move || catalog.set_managed_plugin(&plugin_id, false, true, &expected_revision)).await
}
