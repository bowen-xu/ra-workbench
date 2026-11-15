use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::import_jobs::{ImportJobsRequestV1, ImportJobsResultV1};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn apply_runtime_literature_import_jobs(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    request: ImportJobsRequestV1,
) -> NativeResult<ImportJobsResultV1> {
    service
        .apply_import_jobs(store.shared_agent_connection(), package_key, request)
        .await
}
