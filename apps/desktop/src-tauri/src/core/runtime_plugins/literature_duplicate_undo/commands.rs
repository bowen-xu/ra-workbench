use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn undo_runtime_merge_operation(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    operation_id: String,
) -> NativeResult<()> {
    service
        .undo_merge_operation(store.shared_agent_connection(), package_key, operation_id)
        .await
}
