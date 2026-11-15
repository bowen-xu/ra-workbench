use super::CodexCatalogVersions;
use crate::{agent::model::AgentModelResolver, ai::error::AiCommandError};
use tauri::State;

#[tauri::command]
pub async fn get_codex_catalog_versions(
    models: State<'_, AgentModelResolver>,
    refresh: Option<bool>,
) -> Result<CodexCatalogVersions, AiCommandError> {
    models
        .codex_catalog_versions(refresh.unwrap_or(false))
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn select_codex_catalog_version(
    models: State<'_, AgentModelResolver>,
    version: String,
) -> Result<CodexCatalogVersions, AiCommandError> {
    models
        .select_codex_catalog_version(version)
        .await
        .map_err(Into::into)
}
