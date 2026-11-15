//! Original preparation commands with explicit static or Native composition.
use crate::{
    ai::{
        error::{AiCommandError, AiError},
        paper_reading_contracts::*,
        reading_support::PaperReadingImages,
    },
    storage::AppStore,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tauri::State;
#[tauri::command]
pub(crate) async fn prepare_paper_ai_reading_context(
    store: State<'_, AppStore>,
    paper_id: String,
    attachment_ids: Vec<String>,
) -> Result<PaperAiReadingPrepareResult, Value> {
    prepare_reading_context(store.inner(), paper_id, attachment_ids).await
}
pub(crate) async fn prepare_reading_context(
    store: &AppStore,
    paper: String,
    attachments: Vec<String>,
) -> Result<PaperAiReadingPrepareResult, Value> {
    if let Some(binding) = store.native_note_trace() {
        return binding
            .service
            .prepare_paper_reading_context(
                store.clone(),
                binding.package_key.clone(),
                paper,
                attachments,
            )
            .await;
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(AiError::from)?;
        (store.require_paper_reading_source()?.prepare)(&db, &paper, &attachments)
    })
    .await
    .map_err(|_| json!(AiCommandError::from(AiError::TemporaryUnavailable)))?
    .map_err(|error| json!(AiCommandError::from(error)))
}
fn prepare_error(error: AiError) -> PaperContextError {
    match error {
        AiError::InvalidInput(_) => PaperContextError::InvalidInput,
        _ => PaperContextError::Unavailable,
    }
}
#[tauri::command]
pub(crate) async fn agent_prepare_paper_context(
    store: State<'_, AppStore>,
    request: PrepareRequest,
) -> Result<Preparation, Value> {
    prepare_context(store.inner(), request).await
}
pub(crate) async fn prepare_context(
    store: &AppStore,
    request: PrepareRequest,
) -> Result<Preparation, Value> {
    request.validate().map_err(|error| json!(error))?;
    if let Some(binding) = store.native_note_trace() {
        return binding
            .service
            .prepare_paper_context(store.clone(), binding.package_key.clone(), request)
            .await;
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|_| PaperContextError::Unavailable)?;
        let prepared = (store
            .require_paper_reading_source()
            .map_err(prepare_error)?
            .build)(&db, &request.paper_id, &request.attachment_ids)
        .map_err(prepare_error)?;
        if !(store
            .require_paper_reading_source()
            .map_err(prepare_error)?
            .snapshot_ready)(&prepared)
        {
            return Err(PaperContextError::Conflict);
        }
        let reading = prepared.into_reading_result(None, None);
        let assets = if request.image_assets.is_empty() {
            BTreeMap::new()
        } else {
            let assets = (store
                .require_paper_reading_source()
                .map_err(prepare_error)?
                .save_images)(
                &PaperReadingImages::new(&db, &store), &request.image_assets
            )
            .map_err(prepare_error)?;
            (store
                .require_paper_reading_source()
                .map_err(prepare_error)?
                .validate_images)(&reading, &assets)
            .map_err(prepare_error)?;
            assets
        };
        let snapshot = (store
            .require_paper_reading_source()
            .map_err(prepare_error)?
            .persist)(&mut db, &request.paper_id, &reading, &assets)
        .map_err(prepare_error)?;
        Ok(Preparation {
            schema_version: 1,
            paper_id: snapshot.paper_id,
            snapshot_id: snapshot.id,
            context_hash: snapshot.context_hash,
        })
    })
    .await
    .map_err(|_| json!(PaperContextError::Unavailable))?
    .map_err(|error| json!(error))
}
