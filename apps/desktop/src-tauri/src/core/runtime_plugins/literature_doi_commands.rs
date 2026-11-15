//! The selected package receives typed DOI inputs, never a client or Store.
use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::doi_metadata::MetadataPreviewV1;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn preview_runtime_doi_metadata(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    doi: String,
    item_id: Option<String>,
) -> NativeResult<MetadataPreviewV1> {
    service
        .preview_doi(store.shared_agent_connection(), package_key, doi, item_id)
        .await
}
