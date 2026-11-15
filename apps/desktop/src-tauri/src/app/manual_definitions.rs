//! Existing manual definition/history IPC selects the original static or explicit Native route.
use crate::{
    ai::{error::AiError, knowledge_contracts::CaptureCommandError},
    runtime_plugins::literature_trace::service::LiteratureTraceService,
    storage::AppStore,
};
use research_workbench_native_sdk::knowledge_promotion::{
    manual::{DefinitionClaimDraft, ResearchObjectRevision},
    models::DefinitionClaim,
};
use std::sync::Arc;
#[tauri::command]
pub(crate) async fn save_definition_claim(
    store: tauri::State<'_, AppStore>,
    draft: DefinitionClaimDraft,
) -> Result<DefinitionClaim, CaptureCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .save_definition(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }

    {
        let _ = draft;
        Err(AiError::CapabilityUnavailable("literature_library.concepts").into())
    }
}
#[tauri::command]
pub(crate) async fn list_definition_claims(
    store: tauri::State<'_, AppStore>,
    concept_id: String,
) -> Result<Vec<DefinitionClaim>, CaptureCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .list_definitions(store, binding.package_key, concept_id)
            .await
            .map_err(Into::into);
    }

    {
        let _ = concept_id;
        Err(AiError::CapabilityUnavailable("literature_library.concepts").into())
    }
}
#[tauri::command]
pub(crate) async fn list_research_object_revisions(
    store: tauri::State<'_, AppStore>,
    object_kind: String,
    object_id: String,
) -> Result<Vec<ResearchObjectRevision>, CaptureCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .list_knowledge_revisions(store, binding.package_key, object_kind, object_id)
            .await
            .map_err(Into::into);
    }

    {
        let _ = (object_kind, object_id);
        Err(AiError::CapabilityUnavailable("literature_library.concepts").into())
    }
}
#[tauri::command]
pub(crate) async fn save_runtime_definition_claim(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: DefinitionClaimDraft,
) -> Result<DefinitionClaim, CaptureCommandError> {
    service
        .save_definition(store.inner().clone(), package_key, draft)
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub(crate) async fn list_runtime_definition_claims(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    concept_id: String,
) -> Result<Vec<DefinitionClaim>, CaptureCommandError> {
    service
        .list_definitions(store.inner().clone(), package_key, concept_id)
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub(crate) async fn list_runtime_research_object_revisions(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    object_kind: String,
    object_id: String,
) -> Result<Vec<ResearchObjectRevision>, CaptureCommandError> {
    service
        .list_knowledge_revisions(store.inner().clone(), package_key, object_kind, object_id)
        .await
        .map_err(Into::into)
}
