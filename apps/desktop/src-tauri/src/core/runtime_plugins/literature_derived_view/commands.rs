use super::{DerivedRequest, DerivedResult};
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::derived_view::{DerivedResearchView, DerivedResearchViewDraft};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn list_runtime_derived_research_views(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    project_id: String,
) -> NativeResult<Vec<DerivedResearchView>> {
    match service
        .derived_views(
            store.shared_agent_connection(),
            package_key,
            DerivedRequest::List { project_id },
        )
        .await?
    {
        DerivedResult::Views(value) => Ok(value),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn save_runtime_derived_research_view(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: DerivedResearchViewDraft,
) -> NativeResult<DerivedResearchView> {
    match service
        .derived_views(
            store.shared_agent_connection(),
            package_key,
            DerivedRequest::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        DerivedResult::View(value) => Ok(*value),
        _ => Err(mismatch()),
    }
}
fn mismatch() -> Box<NativeHostError> {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "派生视图结果与本次调用不匹配。",
    )
}
