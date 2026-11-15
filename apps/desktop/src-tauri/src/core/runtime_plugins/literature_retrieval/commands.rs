//! Original command names choose the explicitly assembled source.
use super::{failure, manager::ConceptRetrievalManager};
use crate::{
    ai::{EmbeddingAccess, knowledge_contracts::ConceptRetrievalError},
    storage::AppStore,
};
use research_workbench_native_sdk::concept_retrieval::{
    ConceptRetrievalPreview, ConceptRetrievalPreviewDraft, validate_trace_id,
};
use serde_json::Value;
use tauri::State;

#[tauri::command]
pub(crate) async fn preview_concept_retrieval(
    store: State<'_, AppStore>,
    embedding: State<'_, EmbeddingAccess>,
    manager: State<'_, ConceptRetrievalManager>,
    draft: ConceptRetrievalPreviewDraft,
) -> Result<ConceptRetrievalPreview, Value> {
    preview_selected(
        store.inner().clone(),
        embedding.inner().clone(),
        manager.inner().clone(),
        draft,
    )
    .await
}
pub(crate) async fn preview_selected(
    store: AppStore,
    embedding: EmbeddingAccess,
    manager: ConceptRetrievalManager,
    draft: ConceptRetrievalPreviewDraft,
) -> Result<ConceptRetrievalPreview, Value> {
    let client = draft.client_request_id.clone();
    let registration = manager.register_owned(&client).map_err(failure::original)?;
    let signal = registration.signal();
    tokio::spawn(async move {
        let _registration = registration;
        if let Some(binding) = store.native_note_trace().cloned() {
            return binding
                .service
                .preview_concept_retrieval(store, binding.package_key, embedding, draft, signal)
                .await;
        }
        let source = store
            .retrieval_preview_source()
            .ok_or_else(|| failure::original(ConceptRetrievalError::WorkerUnavailable))?;
        (source.preview)(store, embedding, draft, signal)
            .await
            .map_err(failure::original)
    })
    .await
    .map_err(|_| {
        failure::Error::Unsettled.into_wire(
            Some(&client),
            None,
            "execution",
            crate::hbs_storage::boundary::HbsOutcome::Unknown,
        )
    })?
}
#[tauri::command]
pub(crate) fn cancel_concept_retrieval_preview(
    manager: State<'_, ConceptRetrievalManager>,
    client_request_id: String,
) -> Result<bool, Value> {
    manager
        .cancel(&client_request_id)
        .map_err(failure::original)
}
#[tauri::command]
pub(crate) async fn get_concept_retrieval_trace(
    store: State<'_, AppStore>,
    trace_id: String,
) -> Result<ConceptRetrievalPreview, Value> {
    read_selected(store.inner().clone(), trace_id).await
}
pub(crate) async fn read_selected(
    store: AppStore,
    trace: String,
) -> Result<ConceptRetrievalPreview, Value> {
    validate_trace_id(&trace).map_err(|message| {
        failure::original(ConceptRetrievalError::InvalidInput(message.into()))
    })?;
    let trace = trace.trim().to_owned();
    let requested = trace.clone();
    tokio::spawn(async move {
        if let Some(binding) = store.native_note_trace().cloned() {
            return binding
                .service
                .read_concept_retrieval_trace(store, binding.package_key, trace)
                .await;
        }
        let source = store
            .retrieval_preview_source()
            .ok_or_else(|| failure::original(ConceptRetrievalError::WorkerUnavailable))?;
        tokio::task::spawn_blocking(move || {
            let db = store.connection().map_err(ConceptRetrievalError::from)?;
            (source.read)(&db, &trace)
        })
        .await
        .map_err(|_| failure::original(ConceptRetrievalError::WorkerUnavailable))?
        .map_err(failure::original)
    })
    .await
    .map_err(|_| {
        failure::Error::Unsettled.into_wire(
            None,
            Some(&requested),
            "source-read",
            crate::hbs_storage::boundary::HbsOutcome::Unknown,
        )
    })?
}
