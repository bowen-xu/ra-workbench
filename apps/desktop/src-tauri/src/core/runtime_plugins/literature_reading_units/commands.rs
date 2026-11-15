use super::{ReadingUnitRequest, ReadingUnitResult};
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::reading_units::{
    StructuredReadingUnit, StructuredReadingUnitDraft,
};
use std::sync::Arc;
#[tauri::command]
pub(crate) async fn list_runtime_structured_reading_units(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    project_id: Option<String>,
    paper_id: Option<String>,
) -> NativeResult<Vec<StructuredReadingUnit>> {
    match service
        .reading_units(
            store.shared_agent_connection(),
            package_key,
            ReadingUnitRequest::List {
                project_id,
                paper_id,
            },
        )
        .await?
    {
        ReadingUnitResult::Units(v) => Ok(v),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn save_runtime_structured_reading_unit(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: StructuredReadingUnitDraft,
) -> NativeResult<StructuredReadingUnit> {
    match service
        .reading_units(
            store.shared_agent_connection(),
            package_key,
            ReadingUnitRequest::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        ReadingUnitResult::Unit(v) => Ok(*v),
        _ => Err(mismatch()),
    }
}
fn mismatch() -> Box<NativeHostError> {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "阅读单元结果种类与本次调用不匹配。",
    )
}
