//! Existing preview command with explicit static or independently built composition.
use crate::{
    ai::paper_reading_contracts::{PaperContextError, PaperContextPreview, PaperContextRequest},
    storage::AppStore,
};
use serde_json::{Value, json};
use tauri::State;
#[tauri::command]
pub(crate) async fn agent_preview_paper_context(
    store: State<'_, AppStore>,
    request: PaperContextRequest,
) -> Result<PaperContextPreview, Value> {
    preview_context(store.inner(), request).await
}
pub(crate) async fn preview_context(
    store: &AppStore,
    request: PaperContextRequest,
) -> Result<PaperContextPreview, Value> {
    request.validate().map_err(|error| json!(error))?;
    if let Some(binding) = store.native_note_trace() {
        return binding
            .service
            .preview_paper_context(store.clone(), binding.package_key.clone(), request)
            .await;
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|_| PaperContextError::Unavailable)?;
        let tx = db
            .transaction()
            .map_err(|_| PaperContextError::Unavailable)?;
        let source = store
            .require_paper_reading_source()
            .map_err(|_| PaperContextError::Unavailable)?;
        (source.preview)(&tx, &request)
    })
    .await
    .map_err(|_| json!(PaperContextError::Unavailable))?
    .map_err(|error| json!(error))
}
