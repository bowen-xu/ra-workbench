use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::{nal_mutation::NalMutation, nal_state::NalState};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn mutate_runtime_nal_network(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    mutation: NalMutation,
    expected_revision: i64,
) -> NativeResult<NalState> {
    service
        .inner()
        .mutate_nal_network(
            store.shared_agent_connection(),
            package_key,
            mutation,
            expected_revision,
        )
        .await
}
