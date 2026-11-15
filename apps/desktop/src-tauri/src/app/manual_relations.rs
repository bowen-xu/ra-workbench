//! Original relation IPC selects trusted static composition or the explicit Native package.
use crate::ai::knowledge_contracts::KnowledgeRouteError as RelationCommandError;
use crate::runtime_plugins::native::NativeProfile;
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::knowledge_promotion::{
    models::RelationNodeSummary,
    relation::{RelationAssertion, RelationAssertionDraft},
};
use std::sync::Arc;
#[tauri::command]
pub(crate) async fn search_relation_nodes(
    store: tauri::State<'_, AppStore>,
    query: String,
) -> Result<Vec<RelationNodeSummary>, RelationCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .search_relation_nodes(store, binding.package_key, query)
            .await
            .map_err(Into::into);
    }

    {
        let _ = query;
        Err(RelationCommandError::Native(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "capability_unavailable",
            "literature_library.concepts 不可用。",
        )))
    }
}
#[tauri::command]
pub(crate) async fn search_runtime_relation_nodes(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    query: String,
) -> NativeResult<Vec<RelationNodeSummary>> {
    service
        .search_relation_nodes(store.inner().clone(), package_key, query)
        .await
}
#[tauri::command]
pub(crate) async fn save_relation_assertion(
    store: tauri::State<'_, AppStore>,
    draft: RelationAssertionDraft,
) -> Result<RelationAssertion, RelationCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .save_relation(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }

    {
        let _ = draft;
        Err(RelationCommandError::Native(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "capability_unavailable",
            "literature_library.concepts 不可用。",
        )))
    }
}
#[tauri::command]
pub(crate) async fn save_runtime_relation_assertion(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: RelationAssertionDraft,
) -> NativeResult<RelationAssertion> {
    service
        .save_relation(store.inner().clone(), package_key, draft)
        .await
}
#[tauri::command]
pub(crate) async fn list_relation_assertions(
    store: tauri::State<'_, AppStore>,
    project_id: Option<String>,
) -> Result<Vec<RelationAssertion>, RelationCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .list_relations(store, binding.package_key, project_id)
            .await
            .map_err(Into::into);
    }

    {
        let _ = project_id;
        Err(RelationCommandError::Native(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "capability_unavailable",
            "literature_library.concepts 不可用。",
        )))
    }
}
#[tauri::command]
pub(crate) async fn list_runtime_relation_assertions(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    project_id: Option<String>,
) -> NativeResult<Vec<RelationAssertion>> {
    service
        .list_relations(store.inner().clone(), package_key, project_id)
        .await
}
