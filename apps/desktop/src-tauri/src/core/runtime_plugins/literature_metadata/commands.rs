use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::metadata::{MetadataRequestV1, MetadataResultV1};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn apply_runtime_literature_metadata(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    request: MetadataRequestV1,
) -> NativeResult<MetadataResultV1> {
    service
        .apply_metadata(store.shared_agent_connection(), package_key, request)
        .await
}
