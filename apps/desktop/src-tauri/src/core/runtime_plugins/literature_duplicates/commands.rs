use super::{ReviewRequest, ReviewResult};
use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::duplicate_review::{
    DuplicateCandidate, DuplicateDecisionRequestV1, MergeOperation,
};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn list_runtime_duplicate_candidates(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
) -> NativeResult<Vec<DuplicateCandidate>> {
    match service
        .review_duplicates(
            store.shared_agent_connection(),
            package_key,
            ReviewRequest::Candidates,
        )
        .await?
    {
        ReviewResult::Candidates(items) => Ok(items),
        _ => Err(super::super::native::NativeHostError::for_profile(
            super::super::native::NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "重复候选结果种类不匹配。",
        )),
    }
}

#[tauri::command]
pub(crate) async fn list_runtime_merge_operations(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
) -> NativeResult<Vec<MergeOperation>> {
    match service
        .review_duplicates(
            store.shared_agent_connection(),
            package_key,
            ReviewRequest::History,
        )
        .await?
    {
        ReviewResult::History(items) => Ok(items),
        _ => Err(super::super::native::NativeHostError::for_profile(
            super::super::native::NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "合并历史结果种类不匹配。",
        )),
    }
}

#[tauri::command]
pub(crate) async fn decide_runtime_duplicate_candidate(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    candidate_id: String,
    decision: String,
) -> NativeResult<()> {
    match service
        .review_duplicates(
            store.shared_agent_connection(),
            package_key,
            ReviewRequest::Decide(DuplicateDecisionRequestV1 {
                candidate_id,
                decision,
            }),
        )
        .await?
    {
        ReviewResult::Decided => Ok(()),
        _ => Err(super::super::native::NativeHostError::for_profile(
            super::super::native::NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "重复项审核结果种类不匹配。",
        )),
    }
}
