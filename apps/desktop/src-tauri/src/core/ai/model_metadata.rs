use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use reqwest::{Client, redirect::Policy};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::{
    error::AiError,
    model::{LlmModelDescriptor, LlmProviderModels},
};

const MODELS_URL: &str = "https://models.opencode.ai/api.json";
const CACHE_FILE: &str = "llm-model-metadata/models-opencode-ai.json";
const CACHE_FRESHNESS: Duration = Duration::from_secs(5 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedContextWindow {
    pub(crate) tokens: i64,
    pub(crate) source: String,
}

pub(crate) async fn enrich_models(
    app: &AppHandle,
    mut provider_models: LlmProviderModels,
) -> LlmProviderModels {
    let catalog = load_catalog(app).await;
    if let Some(catalog) = catalog.as_ref() {
        for model in &mut provider_models.models {
            apply_context_window(&provider_models.provider_id, model, catalog);
        }
    }
    provider_models
}

fn apply_context_window(provider_id: &str, model: &mut LlmModelDescriptor, catalog: &Value) {
    if model.context_window_tokens.is_some() {
        return;
    }
    if let Some(resolved) = resolve_context_window(provider_id, &model.id, catalog) {
        model.context_window_tokens = Some(resolved.tokens);
        model.context_window_source = Some(resolved.source);
    }
}

async fn load_catalog(app: &AppHandle) -> Option<Value> {
    let path = cache_path(app)?;
    if cache_is_fresh(&path)
        && let Ok(catalog) = read_catalog(&path)
    {
        return Some(catalog);
    }
    match fetch_catalog().await {
        Ok((bytes, catalog)) => {
            let _ = write_cache_atomically(&path, &bytes);
            Some(catalog)
        }
        Err(_) => read_catalog(&path).ok(),
    }
}

async fn fetch_catalog() -> Result<(Vec<u8>, Value), AiError> {
    let client = Client::builder()
        .redirect(Policy::none())
        .build()
        .map_err(|_| AiError::TemporaryUnavailable)?;
    let mut last_error = AiError::TemporaryUnavailable;
    for _ in 0..=2 {
        match client.get(MODELS_URL).timeout(REQUEST_TIMEOUT).send().await {
            Ok(response) if response.status().is_success() => {
                if response
                    .content_length()
                    .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
                {
                    return Err(AiError::Provider("invalid_response"));
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|_| AiError::Provider("invalid_response"))?;
                if bytes.len() > MAX_RESPONSE_BYTES {
                    return Err(AiError::Provider("invalid_response"));
                }
                let catalog: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| AiError::Provider("invalid_response"))?;
                if !catalog_schema_valid(&catalog) {
                    return Err(AiError::Provider("invalid_response"));
                }
                return Ok((bytes.to_vec(), catalog));
            }
            Ok(_) => last_error = AiError::TemporaryUnavailable,
            Err(error) if error.is_timeout() => last_error = AiError::Timeout,
            Err(_) => last_error = AiError::TemporaryUnavailable,
        }
    }
    Err(last_error)
}

pub(super) fn cache_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_cache_dir()
        .ok()
        .map(|path| path.join(CACHE_FILE))
}

fn cache_is_fresh(path: &Path) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age < CACHE_FRESHNESS)
}

fn read_catalog(path: &Path) -> Result<Value, AiError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| AiError::TemporaryUnavailable)?;
    let mut bytes = Vec::new();
    file.take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| AiError::TemporaryUnavailable)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(AiError::Provider("invalid_response"));
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| AiError::Provider("invalid_response"))?;
    if !catalog_schema_valid(&value) {
        return Err(AiError::Provider("invalid_response"));
    }
    Ok(value)
}

/// Execution does not use the UI's date-suffix alias fallback or fetch metadata.
/// Unknown revisions cannot inherit an undated model's advertised capacity.
pub(super) fn cached_exact_context_window(
    path: &Path,
    model: &research_agent_core::protocol::ModelIdentity,
) -> Option<ResolvedContextWindow> {
    if model.revision.is_some() {
        return None;
    }
    let catalog = read_catalog(path).ok()?;
    let tokens = unique_context(context_candidates(
        &model.provider_id,
        &model.model_id,
        &catalog,
    ))?;
    Some(ResolvedContextWindow {
        tokens,
        source: "models_dev".into(),
    })
}

fn catalog_schema_valid(value: &Value) -> bool {
    value.as_object().is_some_and(|providers| {
        !providers.is_empty()
            && providers.values().all(|provider| {
                provider
                    .as_object()
                    .and_then(|record| record.get("models"))
                    .is_some_and(Value::is_object)
            })
    })
}

fn write_cache_atomically(path: &Path, bytes: &[u8]) -> Result<(), AiError> {
    let parent = path.parent().ok_or(AiError::TemporaryUnavailable)?;
    std::fs::create_dir_all(parent).map_err(|_| AiError::TemporaryUnavailable)?;
    let temporary = parent.join(format!(".models-{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes).map_err(|_| AiError::TemporaryUnavailable)?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = std::fs::remove_file(&temporary);
            Err(AiError::TemporaryUnavailable)
        }
    }
}

fn resolve_context_window(
    provider_id: &str,
    model_id: &str,
    catalog: &Value,
) -> Option<ResolvedContextWindow> {
    let exact = context_candidates(provider_id, model_id.trim(), catalog);
    let tokens = unique_context(exact).or_else(|| {
        let base = strip_date_suffix(model_id.trim())?;
        unique_context(context_candidates(provider_id, base, catalog))
    })?;
    Some(ResolvedContextWindow {
        tokens,
        source: "models_dev".to_owned(),
    })
}

fn context_candidates(provider_id: &str, model_id: &str, catalog: &Value) -> Vec<i64> {
    catalog_provider_ids(provider_id)
        .iter()
        .filter_map(|catalog_provider| {
            catalog
                .get(catalog_provider)?
                .get("models")?
                .get(model_id)?
                .pointer("/limit/context")?
                .as_i64()
                .filter(|tokens| *tokens > 0)
        })
        .collect()
}

fn unique_context(values: Vec<i64>) -> Option<i64> {
    let values = values.into_iter().collect::<BTreeSet<_>>();
    (values.len() == 1).then(|| *values.first().expect("one context value"))
}

fn catalog_provider_ids(provider_id: &str) -> &'static [&'static str] {
    match provider_id {
        "openai" => &["openai"],
        "deepseek" => &["deepseek"],
        "qwen" => &["alibaba", "alibaba-cn", "llmgateway"],
        _ => &[],
    }
}

fn strip_date_suffix(model_id: &str) -> Option<&str> {
    let suffix = model_id.get(model_id.len().checked_sub(11)?..)?;
    let bytes = suffix.as_bytes();
    if bytes.first() != Some(&b'-')
        || bytes.get(5) != Some(&b'-')
        || bytes.get(8) != Some(&b'-')
        || !bytes[1..5].iter().all(u8::is_ascii_digit)
        || !bytes[6..8].iter().all(u8::is_ascii_digit)
        || !bytes[9..11].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    model_id.get(..model_id.len() - 11)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn resolves_exact_provider_model_context() {
        let catalog = json!({
            "openai": { "models": { "gpt": { "limit": { "context": 128000 } } } }
        });
        assert_eq!(
            resolve_context_window("openai", "gpt", &catalog),
            Some(ResolvedContextWindow {
                tokens: 128000,
                source: "models_dev".to_owned()
            })
        );
    }

    #[test]
    fn resolves_unique_qwen_date_alias_and_rejects_ambiguity() {
        let catalog = json!({
            "alibaba": { "models": {} },
            "llmgateway": { "models": {
                "qwen3-vl-flash": { "limit": { "context": 262144 } }
            } }
        });
        assert_eq!(
            resolve_context_window("qwen", "qwen3-vl-flash-2026-01-22", &catalog)
                .map(|value| value.tokens),
            Some(262144)
        );

        let ambiguous = json!({
            "alibaba": { "models": {
                "qwen3-vl-flash": { "limit": { "context": 131072 } }
            } },
            "llmgateway": { "models": {
                "qwen3-vl-flash": { "limit": { "context": 262144 } }
            } }
        });
        assert_eq!(
            resolve_context_window("qwen", "qwen3-vl-flash-2026-01-22", &ambiguous),
            None
        );
    }

    #[test]
    fn cache_round_trips_atomically_and_rejects_invalid_schema() {
        let directory = std::env::temp_dir().join(format!(
            "research-assistant-model-cache-test-{}",
            uuid::Uuid::new_v4()
        ));
        let path = directory.join("catalog.json");
        let valid = br#"{"openai":{"models":{"gpt":{"limit":{"context":128000}}}}}"#;
        write_cache_atomically(&path, valid).expect("atomic cache write");
        assert_eq!(
            resolve_context_window("openai", "gpt", &read_catalog(&path).expect("cache read"))
                .map(|value| value.tokens),
            Some(128000)
        );

        std::fs::write(&path, br#"{"openai":{"models":[]}}"#).expect("invalid cache");
        assert!(read_catalog(&path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }
}
