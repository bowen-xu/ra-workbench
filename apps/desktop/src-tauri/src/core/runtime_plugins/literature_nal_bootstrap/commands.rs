use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::nal_state::NalState;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn read_runtime_nal_network(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
) -> NativeResult<NalState> {
    service
        .inner()
        .read_nal_bootstrap(store.shared_agent_connection(), package_key)
        .await
}
