//! Explicit query-collection commands, without a database or transport grant to plugins.
use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::nal_query::{Content, Stored};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn read_runtime_nal_query_collections(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
) -> NativeResult<Stored> {
    service
        .inner()
        .read_nal_queries(store.shared_agent_connection(), package_key)
        .await
}

#[tauri::command]
pub(crate) async fn save_runtime_nal_query_collections(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    value: Content,
    expected_revision: i64,
) -> NativeResult<Stored> {
    service
        .inner()
        .save_nal_queries(
            store.shared_agent_connection(),
            package_key,
            value,
            expected_revision,
        )
        .await
}
