//! Original algorithm consumers select explicit Native rules or the static compatibility adapter.
use crate::{
    ai::knowledge_contracts::KnowledgeRouteError,
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::knowledge_promotion::algorithms::{
    ConceptAlgorithmRun, SharedFoundationsDraft,
};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn analyze_concept_evolution(
    store: tauri::State<'_, AppStore>,
    concept_id: String,
) -> Result<ConceptAlgorithmRun, KnowledgeRouteError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .concept_evolution(store.inner().clone(), binding.package_key, concept_id)
            .await
            .map_err(Into::into);
    }

    {
        let _ = concept_id;
        Err(unavailable(
            "capability_unavailable",
            "literature_library.concepts 不可用。",
        ))
    }
}

#[tauri::command]
pub(crate) async fn get_concept_algorithm_run(
    store: tauri::State<'_, AppStore>,
    run_id: String,
) -> Result<ConceptAlgorithmRun, KnowledgeRouteError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .concept_algorithm_run(store.inner().clone(), binding.package_key, run_id)
            .await
            .map_err(Into::into);
    }

    {
        let _ = run_id;
        Err(unavailable(
            "capability_unavailable",
            "literature_library.concepts 不可用。",
        ))
    }
}

#[tauri::command]
pub(crate) async fn analyze_runtime_concept_evolution(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    concept_id: String,
) -> NativeResult<ConceptAlgorithmRun> {
    service
        .concept_evolution(store.inner().clone(), package_key, concept_id)
        .await
}

#[tauri::command]
pub(crate) async fn get_runtime_concept_algorithm_run(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    run_id: String,
) -> NativeResult<ConceptAlgorithmRun> {
    service
        .concept_algorithm_run(store.inner().clone(), package_key, run_id)
        .await
}

fn unavailable(code: &'static str, message: &'static str) -> KnowledgeRouteError {
    NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message).into()
}

#[tauri::command]
pub(crate) async fn analyze_shared_foundations(
    store: tauri::State<'_, AppStore>,
    draft: SharedFoundationsDraft,
) -> Result<ConceptAlgorithmRun, KnowledgeRouteError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .shared_foundations(store.inner().clone(), binding.package_key, draft)
            .await
            .map_err(Into::into);
    }

    {
        let _ = draft;
        Err(unavailable(
            "capability_unavailable",
            "literature_library.concepts 不可用。",
        ))
    }
}
#[tauri::command]
pub(crate) async fn analyze_runtime_shared_foundations(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: SharedFoundationsDraft,
) -> NativeResult<ConceptAlgorithmRun> {
    service
        .shared_foundations(store.inner().clone(), package_key, draft)
        .await
}
