//! Original IPC names choose the explicitly composed static or Native source.
use super::{
    IndexOperation, IndexOutput,
    failure::{self, Error},
    manager::SemanticIndexManager,
    model::SemanticIndexError,
};
use crate::{ai::EmbeddingAccess, storage::AppStore};
use research_workbench_native_sdk::semantic_index::{
    EmbeddingJobSummary, IndexGenerationSummary, ReconciliationSummary, SemanticIndexStatus,
};
use serde_json::Value;
use tauri::State;

pub(crate) enum IndexRead {
    Status,
    Job(String),
    Rebuild(String),
}
pub(crate) async fn read_selected(store: AppStore, read: IndexRead) -> Result<IndexOutput, Value> {
    if let Some(binding) = store.native_note_trace().cloned() {
        let (operation, name, id) = match read {
            IndexRead::Status => (IndexOperation::Status, "status", None),
            IndexRead::Job(id) => (IndexOperation::Job(id.clone()), "job", Some(id)),
            IndexRead::Rebuild(id) => (IndexOperation::Rebuild(id.clone()), "rebuild", Some(id)),
        };
        return binding
            .service
            .semantic_index_request(store, binding.package_key, operation)
            .await
            .map_err(|error| error.into_wire(name, id.as_deref()));
    }
    let source = store
        .semantic_index_source()
        .ok_or_else(|| failure::original(SemanticIndexError::WorkerUnavailable))?;
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(SemanticIndexError::from)
            .map_err(failure::original)?;
        match read {
            IndexRead::Status => (source.status)(&mut db).map(IndexOutput::Status),
            IndexRead::Job(id) => (source.job)(&mut db, id).map(IndexOutput::Job),
            IndexRead::Rebuild(id) => (source.rebuild)(&mut db, id).map(IndexOutput::Generation),
        }
        .map_err(static_error)
    })
    .await
    .map_err(|_| Error::Unsettled.into_wire("execution", None))?
}
fn static_error(error: super::model::SemanticIndexCommandError) -> Value {
    serde_json::to_value(error).expect("index command error has only string fields")
}
#[tauri::command]
pub(crate) async fn get_semantic_index_status(
    store: State<'_, AppStore>,
) -> Result<SemanticIndexStatus, Value> {
    match read_selected(store.inner().clone(), IndexRead::Status).await? {
        IndexOutput::Status(value) => Ok(value),
        _ => Err(Error::mismatch().into_wire("status", None)),
    }
}
#[tauri::command]
pub(crate) async fn get_semantic_embedding_job(
    store: State<'_, AppStore>,
    job_id: String,
) -> Result<EmbeddingJobSummary, Value> {
    match read_selected(store.inner().clone(), IndexRead::Job(job_id.clone())).await? {
        IndexOutput::Job(value) => Ok(value),
        _ => Err(Error::mismatch().into_wire("job", Some(&job_id))),
    }
}
pub(crate) async fn retry_selected(
    store: AppStore,
    embedding: EmbeddingAccess,
    manager: SemanticIndexManager,
    representation: String,
) -> Result<EmbeddingJobSummary, Value> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .start_semantic_index_job(
                store,
                binding.package_key,
                embedding,
                manager,
                representation.clone(),
            )
            .await
            .map_err(|error| error.into_wire("retry", Some(&representation)));
    }
    let source = store
        .semantic_index_source()
        .ok_or_else(|| failure::original(SemanticIndexError::WorkerUnavailable))?;
    tokio::task::spawn_blocking(move || {
        (source.retry)(store, embedding, manager, representation).map_err(static_error)
    })
    .await
    .map_err(|_| Error::Unsettled.into_wire("retry", None))?
}
#[tauri::command]
pub(crate) async fn retry_semantic_representation(
    store: State<'_, AppStore>,
    embedding: State<'_, EmbeddingAccess>,
    manager: State<'_, SemanticIndexManager>,
    representation_id: String,
) -> Result<EmbeddingJobSummary, Value> {
    retry_selected(
        store.inner().clone(),
        embedding.inner().clone(),
        manager.inner().clone(),
        representation_id,
    )
    .await
}
#[tauri::command]
pub(crate) fn cancel_semantic_index_job(
    manager: State<'_, SemanticIndexManager>,
    job_id: String,
) -> Result<bool, Value> {
    manager.cancel(&job_id).map_err(failure::original)
}
#[tauri::command]
pub(crate) async fn rebuild_semantic_index(
    store: State<'_, AppStore>,
    embedding_space_id: String,
) -> Result<IndexGenerationSummary, Value> {
    match read_selected(
        store.inner().clone(),
        IndexRead::Rebuild(embedding_space_id.clone()),
    )
    .await?
    {
        IndexOutput::Generation(value) => Ok(value),
        _ => Err(Error::mismatch().into_wire("rebuild", Some(&embedding_space_id))),
    }
}
pub(crate) async fn reconcile_selected(
    store: AppStore,
    manager: SemanticIndexManager,
) -> Result<ReconciliationSummary, Value> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .reconcile_semantic_index(store, binding.package_key, manager)
            .await
            .map_err(|error| error.into_wire("reconcile", None));
    }
    let source = store
        .semantic_index_source()
        .ok_or_else(|| failure::original(SemanticIndexError::WorkerUnavailable))?;
    tokio::spawn((source.reconcile)(store, manager))
        .await
        .map_err(|_| Error::Unsettled.into_wire("reconcile", None))?
        .map_err(static_error)
}
#[tauri::command]
pub(crate) async fn run_semantic_index_reconciliation(
    store: State<'_, AppStore>,
    manager: State<'_, SemanticIndexManager>,
) -> Result<ReconciliationSummary, Value> {
    reconcile_selected(store.inner().clone(), manager.inner().clone()).await
}
