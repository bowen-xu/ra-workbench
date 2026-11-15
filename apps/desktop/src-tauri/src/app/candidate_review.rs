//! Review IPC selects the existing native owner or static App adapter.
use crate::ai::review_support::AiSuggestionItemDecisionDraft;

use crate::{
    ai::{
        error::{AiCommandError, AiError},
        review_support::{AiCandidate, AiReviewDecisionDraft},
    },
    storage::AppStore,
};

use tauri::State;

// Production composition entries also exercise the full boundary on SQL fixtures.

#[tauri::command]
pub(crate) async fn decide_ai_candidate(
    store: State<'_, AppStore>,
    draft: AiReviewDecisionDraft,
) -> Result<AiCandidate, AiCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .decide_analysis_candidate(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }

    {
        let _ = (store, draft);
        Err(AiError::CapabilityUnavailable("literature_library.generation_policy_source").into())
    }
}
#[tauri::command]
pub(crate) async fn decide_ai_suggestion_item(
    store: State<'_, AppStore>,
    draft: AiSuggestionItemDecisionDraft,
) -> Result<AiCandidate, AiCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .decide_suggestion_item(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }

    {
        let _ = (store, draft);
        Err(AiError::CapabilityUnavailable("literature_library.generation_policy_source").into())
    }
}
#[tauri::command]
pub(crate) async fn decide_ai_paper_concept_item(
    store: State<'_, AppStore>,
    draft: AiSuggestionItemDecisionDraft,
) -> Result<AiCandidate, AiCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .decide_paper_concept_item(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }

    {
        let _ = (store, draft);
        Err(AiError::CapabilityUnavailable("literature_library.generation_policy_source").into())
    }
}
