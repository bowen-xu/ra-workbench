//! Existing settings operations for App composition and no-secret index facts.
pub(crate) use super::model::{AiProviderSettings, AiProviderSettingsDraft};
use super::{JsonCredentialStore, error::AiError, settings_repository};
use rusqlite::{Connection, OptionalExtension};
pub(crate) struct ConfiguredEmbeddingSpace {
    pub region: String,
    pub model_id: String,
}
pub(crate) fn configured_space_in(
    db: &Connection,
) -> rusqlite::Result<Option<ConfiguredEmbeddingSpace>> {
    db.query_row(
        "select region, model_id from ai_provider_settings
             where provider_id = 'qwen-embedding' and enabled = 1
               and length(trim(model_id)) > 0",
        [],
        |row| {
            Ok(ConfiguredEmbeddingSpace {
                region: row.get(0)?,
                model_id: row.get(1)?,
            })
        },
    )
    .optional()
}
pub(crate) fn save_in(
    db: &Connection,
    credentials: &JsonCredentialStore,
    draft: AiProviderSettingsDraft,
) -> Result<AiProviderSettings, AiError> {
    settings_repository::save_provider_settings(db, credentials, draft)
}
