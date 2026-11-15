//! Codex models-manager policy adapted at the application's existing auth/I/O ports.
//! Upstream e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0.
#[path = "cache.rs"]
mod cache;
#[path = "commands.rs"]
pub mod commands;
#[path = "model.rs"]
mod model;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[path = "transport.rs"]
mod transport;
#[path = "versions.rs"]
mod versions;
use super::{
    error::AiError,
    model::{LlmCatalogStatus, LlmProviderModels},
};
use crate::agent::authentication::{ChatGptCredentialSource, ChatGptLease};
use cache::{CURRENT_SCHEMA_VERSION, CatalogCache};
use model::CatalogModel;
pub(crate) use model::TruncationPolicy;
use serde::Deserialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;
pub use versions::CodexCatalogVersions;
pub(crate) use versions::is_version as valid_catalog_version;

// Bundled sidecar identity and default catalog version; selection never mutates it.
pub(crate) const CLIENT_VERSION: &str = "0.146.0";
pub(crate) const MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";
const MAX_BYTES: usize = 8 * 1024 * 1024;
const TTL: u64 = 300;
#[derive(Deserialize)]
struct WireCatalog {
    models: Vec<CatalogModel>,
}
pub(crate) struct CodexModelCatalog {
    source: Arc<dyn ChatGptCredentialSource>,
    transport: Arc<dyn transport::CatalogTransport>,
    directory: PathBuf,
    overrides: PathBuf,
    version_settings: PathBuf,
    refresh: tokio::sync::Mutex<()>,
}
pub(crate) struct CatalogSnapshot {
    models: Vec<CatalogModel>,
    status: LlmCatalogStatus,
    account_scope: Option<uuid::Uuid>,
}
pub(super) fn bundled_snapshot() -> Result<CatalogSnapshot, AiError> {
    let models = serde_json::from_str::<WireCatalog>(include_str!("bundled.json"))
        .map_err(|_| AiError::Provider("invalid_response"))?
        .models;
    model::validate(&models)?;
    Ok(CatalogSnapshot {
        models,
        account_scope: None,
        status: LlmCatalogStatus {
            source: "bundled".into(),
            stale: true,
            fetched_at: 0,
        },
    })
}
impl CatalogSnapshot {
    pub(crate) fn account_scope(&self) -> Option<uuid::Uuid> {
        self.account_scope
    }
    pub(crate) fn provider_models(&self) -> LlmProviderModels {
        let visible = self
            .models
            .iter()
            .filter(|model| model.visibility == "list");
        LlmProviderModels {
            provider_id: "openai".into(),
            models: visible
                .enumerate()
                .map(|(index, model)| model.descriptor(index == 0))
                .collect(),
            catalog: Some(self.status.clone()),
        }
    }
    pub(crate) fn model(&self, id: &str) -> Option<&CatalogModel> {
        self.models.iter().find(|model| model.slug == id)
    }
}
impl CodexModelCatalog {
    #[cfg(test)]
    pub(crate) fn cached_fixture(
        source: Arc<dyn ChatGptCredentialSource>,
        directory: PathBuf,
        overrides: PathBuf,
    ) -> Self {
        Self {
            source,
            transport: Arc::new(transport::NoNetwork),
            directory: directory.clone(),
            overrides,
            version_settings: directory.with_extension("versions.json"),
            refresh: tokio::sync::Mutex::new(()),
        }
    }
    pub(crate) fn from_app(
        app: &tauri::AppHandle,
        source: Arc<dyn ChatGptCredentialSource>,
    ) -> Option<Self> {
        Some(Self {
            source,
            transport: Arc::new(transport::HttpCatalogTransport),
            directory: app
                .path()
                .app_cache_dir()
                .ok()?
                .join("llm-model-metadata/codex-models"),
            overrides: app
                .path()
                .app_config_dir()
                .ok()?
                .join("model-context-overrides.json"),
            version_settings: app
                .path()
                .app_config_dir()
                .ok()?
                .join("codex-catalog-versions.json"),
            refresh: tokio::sync::Mutex::new(()),
        })
    }
    pub(crate) async fn load(&self, force: bool) -> Result<CatalogSnapshot, AiError> {
        let _lock = self.refresh.lock().await;
        let version = self.read_versions().await?.selected_version;
        let lease = self
            .source
            .acquire()
            .await
            .map_err(|_| AiError::Provider("authentication"))?;
        let path = self.directory.join(format!("{}.json", lease.scope_id()));
        let read_path = path.clone();
        let previous = tokio::task::spawn_blocking(move || cache::read(&read_path))
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?
            .filter(|cache| cache.client_version == version);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AiError::TemporaryUnavailable)?
            .as_secs();
        let fresh = previous.as_ref().is_some_and(|cache| {
            cache.schema_version == CURRENT_SCHEMA_VERSION
                && now
                    .checked_sub(cache.fetched_at)
                    .is_some_and(|age| age < TTL)
        });
        let (mut cache, source, stale) = if !force && fresh {
            (previous.unwrap(), "cache", false)
        } else {
            // Version 1 discarded policy metadata. Its ETag cannot prove that
            // the sanitized cache contains the complete current response.
            let conditional = previous
                .as_ref()
                .filter(|cache| cache.schema_version == CURRENT_SCHEMA_VERSION);
            match self.transport.fetch(&lease, conditional, &version).await {
                Ok((models, etag)) => {
                    let cache = CatalogCache {
                        schema_version: CURRENT_SCHEMA_VERSION,
                        client_version: version.clone(),
                        fetched_at: now,
                        etag,
                        models,
                    };
                    lease
                        .check()
                        .map_err(|_| AiError::Provider("authentication"))?;
                    let saved = cache.clone();
                    // A cache is an optimization; a failed disk write does not
                    // discard successfully authenticated remote metadata.
                    let _ = tokio::task::spawn_blocking(move || cache::write(&path, &saved)).await;
                    (cache, "remote", false)
                }
                Err(AiError::Provider("authentication")) => {
                    return Err(AiError::Provider("authentication"));
                }
                Err(_) => match previous {
                    Some(cache) => (cache, "cache", true),
                    None => (
                        CatalogCache {
                            schema_version: CURRENT_SCHEMA_VERSION,
                            client_version: version.clone(),
                            fetched_at: 0,
                            etag: None,
                            models: bundled_snapshot()?.models,
                        },
                        "bundled",
                        true,
                    ),
                },
            }
        };
        lease
            .check()
            .map_err(|_| AiError::Provider("authentication"))?;
        let override_path = self.overrides.clone();
        let overrides = tokio::task::spawn_blocking(move || model::read_overrides(&override_path))
            .await
            .map_err(|_| AiError::TemporaryUnavailable)??;
        lease
            .check()
            .map_err(|_| AiError::Provider("authentication"))?;
        cache.models = cache
            .models
            .into_iter()
            .map(|model| {
                let value = overrides.get(&model.slug);
                model.with_override(value)
            })
            .collect();
        cache.models.sort_by_key(|model| model.priority);
        Ok(CatalogSnapshot {
            models: cache.models,
            account_scope: Some(lease.scope_id()),
            status: LlmCatalogStatus {
                source: source.into(),
                stale,
                fetched_at: cache.fetched_at,
            },
        })
    }
    pub(crate) async fn invalidate(&self) {
        let _lock = self.refresh.lock().await;
        let path = self.directory.clone();
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(path)).await;
    }
}
