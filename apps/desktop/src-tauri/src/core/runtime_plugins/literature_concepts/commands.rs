use super::{ConceptRequest, ConceptResult};
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::concepts::{Concept, ConceptDraft};
use std::sync::Arc;
#[tauri::command]
pub(crate) async fn search_runtime_research_concepts(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    query: String,
) -> NativeResult<Vec<Concept>> {
    match service
        .concepts(
            store.shared_agent_connection(),
            package_key,
            ConceptRequest::Search { query },
        )
        .await?
    {
        ConceptResult::Concepts(values) => Ok(values),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn save_runtime_research_concept(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: ConceptDraft,
) -> NativeResult<Concept> {
    match service
        .concepts(
            store.shared_agent_connection(),
            package_key,
            ConceptRequest::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        ConceptResult::Concept(value) => Ok(*value),
        _ => Err(mismatch()),
    }
}
fn mismatch() -> Box<NativeHostError> {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "概念结果种类与本次调用不匹配。",
    )
}

#[tauri::command]
pub(crate) async fn capture_runtime_research_concept_usage(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: research_workbench_native_sdk::knowledge_promotion::capture::ConceptCaptureDraft,
) -> Result<
    research_workbench_native_sdk::knowledge_promotion::capture::ConceptCaptureResult,
    crate::ai::knowledge_contracts::CaptureCommandError,
> {
    service
        .capture_concept(store.inner().clone(), package_key, draft)
        .await
        .map_err(Into::into)
}
