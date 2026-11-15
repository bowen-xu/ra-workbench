//! App assembly of shared model configuration and optional domain maintenance.

use crate::{
    ai::{
        JsonCredentialStore,
        embedding_settings::{self, AiProviderSettings, AiProviderSettingsDraft},
        error::AiCommandError,
    },
    storage::AppStore,
};
use tauri::State;

#[tauri::command]
pub(crate) fn save_ai_provider_settings(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    draft: AiProviderSettingsDraft,
) -> Result<AiProviderSettings, serde_json::Value> {
    save_selected(&store, &credentials, draft)
}
pub(crate) fn save_selected(
    store: &AppStore,
    credentials: &JsonCredentialStore,
    draft: AiProviderSettingsDraft,
) -> Result<AiProviderSettings, serde_json::Value> {
    let db = store
        .connection()
        .map_err(AiCommandError::from)
        .map_err(command_error)?;
    let settings = embedding_settings::save_in(&db, credentials, draft)
        .map_err(AiCommandError::from)
        .map_err(command_error)?;
    if settings.provider_id == "qwen-embedding" && settings.enabled {
        if let Some(binding) = store.native_note_trace() {
            binding
                .service
                .semantic_index_backfill_in(&db, &binding.package_key)
                .map_err(|error| error.into_wire("provider-backfill", None))?;
            return Ok(settings);
        }
    }
    Ok(settings)
}

fn command_error(error: AiCommandError) -> serde_json::Value {
    serde_json::to_value(error).expect("AI command errors contain serializable diagnostic fields")
}
