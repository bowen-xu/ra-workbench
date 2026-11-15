use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::duplicate_review::MergeOperation;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn merge_runtime_duplicate_candidate(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    candidate_id: String,
    survivor_item_id: String,
) -> NativeResult<MergeOperation> {
    service
        .merge_duplicate_candidate(
            store.shared_agent_connection(),
            package_key,
            candidate_id,
            survivor_item_id,
        )
        .await
}
