use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::nal_annotation::{NalAnnotation, NalAnnotationRequest};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn ensure_runtime_nal_annotation(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    request: NalAnnotationRequest,
) -> NativeResult<NalAnnotation> {
    service
        .inner()
        .ensure_nal_annotation(store.shared_agent_connection(), package_key, request)
        .await
}
