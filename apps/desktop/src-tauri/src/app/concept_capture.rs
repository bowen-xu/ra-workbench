//! Original capture IPC selects the explicit Native binding or trusted static adapter.
use crate::{
    ai::{error::AiError, knowledge_contracts::CaptureCommandError},
    storage::AppStore,
};
use research_workbench_native_sdk::knowledge_promotion::capture::{
    ConceptCaptureDraft, ConceptCaptureResult,
};

#[tauri::command]
pub(crate) async fn capture_concept_usage(
    store: tauri::State<'_, AppStore>,
    draft: ConceptCaptureDraft,
) -> Result<ConceptCaptureResult, CaptureCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .capture_concept(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }

    {
        let _ = draft;
        Err(AiError::CapabilityUnavailable("literature_library.concepts").into())
    }
}
