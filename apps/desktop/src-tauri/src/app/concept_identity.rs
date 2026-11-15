//! Four original identity commands select static composition or the explicit Native package.
use crate::{
    ai::knowledge_contracts::KnowledgeRouteError,
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::knowledge_promotion::identity::*;
use std::sync::Arc;
async fn selected(
    store: AppStore,
    request: IdentityRequestV1,
) -> Result<IdentityOutputV1, KnowledgeRouteError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .concept_identity(store, binding.package_key, request)
            .await
            .map_err(Into::into);
    }

    {
        let _ = request;
        Err(KnowledgeRouteError::Native(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "capability_unavailable",
            "literature_library.concepts 不可用。",
        )))
    }
}
fn mismatch() -> KnowledgeRouteError {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "概念身份回复种类不匹配。",
    )
    .into()
}
#[tauri::command]
pub(crate) async fn preview_concept_merge(
    store: tauri::State<'_, AppStore>,
    draft: ConceptMergeDraft,
) -> Result<ConceptMergeImpact, KnowledgeRouteError> {
    match selected(store.inner().clone(), IdentityRequestV1::Preview { draft }).await? {
        IdentityOutputV1::Impact { impact } => Ok(impact),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn merge_concepts(
    store: tauri::State<'_, AppStore>,
    draft: ConceptMergeDraft,
) -> Result<ConceptIdentityEvent, KnowledgeRouteError> {
    match selected(store.inner().clone(), IdentityRequestV1::Merge { draft }).await? {
        IdentityOutputV1::Event { event } => Ok(event),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn split_concepts(
    store: tauri::State<'_, AppStore>,
    draft: ConceptSplitDraft,
) -> Result<ConceptIdentityEvent, KnowledgeRouteError> {
    match selected(store.inner().clone(), IdentityRequestV1::Split { draft }).await? {
        IdentityOutputV1::Event { event } => Ok(event),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn list_concept_identity_events(
    store: tauri::State<'_, AppStore>,
) -> Result<Vec<ConceptIdentityEvent>, KnowledgeRouteError> {
    match selected(store.inner().clone(), IdentityRequestV1::List {}).await? {
        IdentityOutputV1::Events { events } => Ok(events),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn apply_runtime_concept_identity(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    request: IdentityRequestV1,
) -> NativeResult<IdentityOutputV1> {
    service
        .concept_identity(store.inner().clone(), package_key, request)
        .await
}
