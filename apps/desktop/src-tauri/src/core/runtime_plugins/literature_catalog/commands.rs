//! Finite catalog reads; the selected package never receives Store or SQL access.
use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::catalog::{CatalogItem, CatalogRequest};
use research_workbench_native_sdk::catalog_save::CatalogDraft;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn list_runtime_literature(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    query: Option<String>,
) -> NativeResult<Vec<CatalogItem>> {
    service
        .catalog(
            store.shared_agent_connection(),
            package_key,
            CatalogRequest::List { query },
        )
        .await
}

#[tauri::command]
pub(crate) async fn list_runtime_trashed_literature(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    query: Option<String>,
) -> NativeResult<Vec<CatalogItem>> {
    service
        .inner()
        .list_trashed_catalog(store.shared_agent_connection(), package_key, query)
        .await
}

#[tauri::command]
pub(crate) async fn get_runtime_literature_item(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    id: String,
) -> NativeResult<CatalogItem> {
    // The service checks the one-result invariant against the request identity.
    let mut items = service
        .catalog(
            store.shared_agent_connection(),
            package_key,
            CatalogRequest::Get { id },
        )
        .await?;
    Ok(items.remove(0))
}

#[tauri::command]
pub(crate) async fn save_runtime_literature_item(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: CatalogDraft,
) -> NativeResult<CatalogItem> {
    service
        .inner()
        .save_catalog(store.shared_agent_connection(), package_key, draft)
        .await
}
