use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::literature_lifecycle::LifecycleRequestV1;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn apply_runtime_literature_lifecycle(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    request: LifecycleRequestV1,
) -> NativeResult<()> {
    service
        .apply_literature_lifecycle(store.shared_agent_connection(), package_key, request)
        .await
}
