use std::collections::{BTreeMap, BTreeSet};

use reqwest::{Client, StatusCode, redirect::Policy};
use rusqlite::{Connection, Transaction, params};
use serde::Deserialize;

use super::{
    credential::{JsonCredentialStore, ProviderSecret},
    error::AiError,
    model::{
        AiProviderSettings, LlmConfiguration, LlmModelDescriptor, LlmModelRoute,
        LlmModelRouteDraft, LlmModelSelection, LlmModelSelectionDraft, LlmProviderConfig,
        LlmProviderConfigDraft, LlmProviderModels,
    },
};

const CATALOG_JSON: &str =
    include_str!("../../../../../../packages/core/src/ai/llm-provider-defaults.json");
const MAX_MODEL_RESPONSE_BYTES: usize = 1024 * 1024;

const DEFAULT_ROUTES: &[(&str, &str)] = &[
    ("text", "文本模型"),
    ("embedding", "Embedding 模型"),
    ("vision", "视觉模型"),
];

const USE_CASE_ROUTES: &[(&str, &str, &str)] = &[
    ("selection_explanation", "解释所选内容", "text"),
    ("paper_profile_classification", "生成文献画像", "text"),
    ("paper_overview", "生成论文概览", "vision"),
    ("paper_reading_chat", "论文 AI 阅读对话", "vision"),
    ("paper_suggestions", "生成阅读建议", "text"),
    ("reference_extraction", "提取参考文献", "text"),
    ("matrix_synthesis", "多文献综合", "text"),
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderCatalog {
    schema_version: u32,
    providers: Vec<ProviderCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderCatalogEntry {
    id: String,
    display_name: String,
    auth_mode: String,
    protocol: String,
    default_base_urls: Vec<String>,
    model_kinds: Vec<String>,
}

#[derive(Debug)]
struct StoredProviderConfig {
    enabled: bool,
    base_url: String,
    timeout_ms: i64,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

fn catalog() -> Result<ProviderCatalog, AiError> {
    let catalog: ProviderCatalog = serde_json::from_str(CATALOG_JSON)
        .map_err(|_| AiError::InvalidInput("内置 Provider 目录格式无效".to_owned()))?;
    if catalog.schema_version != 1 || catalog.providers.is_empty() {
        return Err(AiError::InvalidInput(
            "内置 Provider 目录版本无效".to_owned(),
        ));
    }
    let mut ids = BTreeSet::new();
    for provider in &catalog.providers {
        if !ids.insert(provider.id.as_str())
            || provider.model_kinds.is_empty()
            || !matches!(provider.auth_mode.as_str(), "api-key" | "chatgpt-login")
            || !matches!(
                provider.protocol.as_str(),
                "openai-compatible" | "codex-cli"
            )
            || (provider.auth_mode == "api-key" && provider.default_base_urls.is_empty())
            || (provider.auth_mode == "chatgpt-login" && !provider.default_base_urls.is_empty())
        {
            return Err(AiError::InvalidInput(
                "内置 Provider 目录内容无效".to_owned(),
            ));
        }
        for base_url in &provider.default_base_urls {
            validate_base_url(&provider.id, base_url)?;
        }
    }
    Ok(catalog)
}

pub(crate) fn get_configuration(
    connection: &Connection,
    credentials: &JsonCredentialStore,
) -> Result<LlmConfiguration, AiError> {
    let catalog = catalog()?;
    let stored = load_stored_providers(connection)?;
    let providers = catalog
        .providers
        .iter()
        .map(|entry| provider_config(entry, stored.get(&entry.id), credentials))
        .collect::<Result<Vec<_>, _>>()?;
    let selections = load_routes(connection)?;
    Ok(LlmConfiguration {
        providers,
        default_models: DEFAULT_ROUTES
            .iter()
            .map(|(route_id, label)| route("default", route_id, label, route_id, &selections))
            .collect(),
        use_cases: USE_CASE_ROUTES
            .iter()
            .map(|(route_id, label, model_kind)| {
                route("use_case", route_id, label, model_kind, &selections)
            })
            .collect(),
    })
}

pub(crate) fn save_provider(
    connection: &Connection,
    credentials: &JsonCredentialStore,
    draft: LlmProviderConfigDraft,
) -> Result<LlmConfiguration, AiError> {
    let catalog = catalog()?;
    let entry = catalog
        .providers
        .iter()
        .find(|provider| provider.id == draft.provider_id)
        .ok_or_else(|| AiError::InvalidInput("未知 LLM Provider".to_owned()))?;
    if !(1_000..=300_000).contains(&draft.timeout_ms) {
        return Err(AiError::InvalidInput(
            "Provider 超时必须在 1–300 秒之间".to_owned(),
        ));
    }
    let configured_base_url = draft.base_url.trim();
    if !configured_base_url.is_empty() {
        validate_base_url(&entry.id, configured_base_url)?;
    }
    if entry.auth_mode == "chatgpt-login" {
        if !draft.base_url.trim().is_empty() {
            return Err(AiError::InvalidInput(
                "ChatGPT 登录 Provider 不接受 Base URL".to_owned(),
            ));
        }
    } else if draft.enabled && !credentials.summary(&entry.id)?.configured {
        return Err(AiError::CredentialMissing);
    }
    connection.execute(
        "update llm_provider_configs
         set enabled = ?2, base_url = ?3, timeout_ms = ?4,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where provider_id = ?1",
        params![
            entry.id,
            draft.enabled,
            configured_base_url,
            draft.timeout_ms
        ],
    )?;
    get_configuration(connection, credentials)
}

pub(crate) fn save_routes(
    connection: &mut Connection,
    credentials: &JsonCredentialStore,
    drafts: Vec<LlmModelRouteDraft>,
) -> Result<LlmConfiguration, AiError> {
    let catalog = catalog()?;
    let mut seen = BTreeSet::new();
    for draft in &drafts {
        if !seen.insert((draft.route_kind.as_str(), draft.route_id.as_str())) {
            return Err(AiError::InvalidInput("模型路由包含重复项目".to_owned()));
        }
        let model_kind = route_model_kind(&draft.route_kind, &draft.route_id)
            .ok_or_else(|| AiError::InvalidInput("未知模型路由".to_owned()))?;
        if let Some(selection) = &draft.selection {
            let provider = catalog
                .providers
                .iter()
                .find(|provider| provider.id == selection.provider_id)
                .ok_or_else(|| AiError::InvalidInput("模型路由引用了未知 Provider".to_owned()))?;
            if !provider.model_kinds.iter().any(|kind| kind == model_kind) {
                return Err(AiError::InvalidInput(format!(
                    "{} 不支持 {model_kind} 模型",
                    provider.display_name
                )));
            }
            if selection.model_id.trim().is_empty() || selection.model_id.trim().len() > 240 {
                return Err(AiError::InvalidInput(
                    "模型 ID 不能为空且不得超过 240 字符".to_owned(),
                ));
            }
        }
    }

    let transaction = connection.transaction()?;
    for draft in drafts {
        persist_route(&transaction, draft)?;
    }
    transaction.commit()?;
    get_configuration(connection, credentials)
}

pub(crate) fn provider_for_model_fetch(
    connection: &Connection,
    credentials: &JsonCredentialStore,
    provider_id: &str,
) -> Result<(LlmProviderConfig, ProviderSecret), AiError> {
    let configuration = get_configuration(connection, credentials)?;
    let provider = configuration
        .providers
        .into_iter()
        .find(|provider| provider.provider_id == provider_id)
        .ok_or_else(|| AiError::InvalidInput("未知 LLM Provider".to_owned()))?;
    let credential = credentials.resolve(provider_id)?;
    Ok((provider, credential))
}

pub(crate) fn selection_for_route(
    connection: &Connection,
    route_kind: &str,
    route_id: &str,
) -> Result<Option<LlmModelSelection>, AiError> {
    load_routes(connection).map(|routes| {
        routes
            .get(&(route_kind.to_owned(), route_id.to_owned()))
            .cloned()
    })
}

pub(crate) async fn fetch_models(
    provider: LlmProviderConfig,
    credential: ProviderSecret,
) -> Result<LlmProviderModels, AiError> {
    let endpoint = format!("{}/models", provider.base_url.trim_end_matches('/'));
    let client = Client::builder()
        .redirect(Policy::none())
        .build()
        .map_err(|_| AiError::TemporaryUnavailable)?;
    let response = client
        .get(endpoint)
        .bearer_auth(credential.expose())
        .timeout(std::time::Duration::from_millis(provider.timeout_ms as u64))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                AiError::Timeout
            } else {
                AiError::TemporaryUnavailable
            }
        })?;
    if !response.status().is_success() {
        return Err(http_error(response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MODEL_RESPONSE_BYTES as u64)
    {
        return Err(AiError::Provider("invalid_response"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AiError::Provider("invalid_response"))?;
    if bytes.len() > MAX_MODEL_RESPONSE_BYTES {
        return Err(AiError::Provider("invalid_response"));
    }
    let payload: ModelsResponse =
        serde_json::from_slice(&bytes).map_err(|_| AiError::Provider("invalid_response"))?;
    let models = payload
        .data
        .into_iter()
        .map(|model| model.id.trim().to_owned())
        .filter(|id| !id.is_empty() && id.len() <= 240)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| LlmModelDescriptor {
            display_name: id.clone(),
            id,
            description: String::new(),
            is_default: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            supports_fast_mode: false,
            context_window_tokens: None,
            context_window_source: None,
        })
        .collect();
    Ok(LlmProviderModels {
        provider_id: provider.provider_id,
        models,
        catalog: None,
    })
}

fn load_stored_providers(
    connection: &Connection,
) -> Result<BTreeMap<String, StoredProviderConfig>, AiError> {
    let mut statement = connection
        .prepare("select provider_id, enabled, base_url, timeout_ms from llm_provider_configs")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            StoredProviderConfig {
                enabled: row.get(1)?,
                base_url: row.get(2)?,
                timeout_ms: row.get(3)?,
            },
        ))
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

fn provider_config(
    entry: &ProviderCatalogEntry,
    stored: Option<&StoredProviderConfig>,
    credentials: &JsonCredentialStore,
) -> Result<LlmProviderConfig, AiError> {
    let stored_base_url = stored.map(|item| item.base_url.trim()).unwrap_or_default();
    let uses_default_base_url = stored_base_url.is_empty();
    let summary = if entry.auth_mode == "api-key" {
        Some(credentials.summary(&entry.id)?)
    } else {
        None
    };
    Ok(LlmProviderConfig {
        provider_id: entry.id.clone(),
        display_name: entry.display_name.clone(),
        auth_mode: entry.auth_mode.clone(),
        protocol: entry.protocol.clone(),
        model_kinds: entry.model_kinds.clone(),
        default_base_urls: entry.default_base_urls.clone(),
        base_url: if uses_default_base_url {
            entry.default_base_urls.first().cloned().unwrap_or_default()
        } else {
            stored_base_url.to_owned()
        },
        uses_default_base_url,
        enabled: stored.is_some_and(|item| item.enabled),
        timeout_ms: stored.map(|item| item.timeout_ms).unwrap_or(60_000),
        credential_configured: summary.as_ref().is_some_and(|value| value.configured),
        credential_mask: summary.and_then(|value| value.masked_value),
    })
}

fn load_routes(
    connection: &Connection,
) -> Result<BTreeMap<(String, String), LlmModelSelection>, AiError> {
    let mut statement = connection
        .prepare("select route_kind, route_id, provider_id, model_id, reasoning_effort from llm_model_routes")?;
    let rows = statement.query_map([], |row| {
        Ok((
            (row.get(0)?, row.get(1)?),
            LlmModelSelection {
                provider_id: row.get(2)?,
                model_id: row.get(3)?,
                reasoning_effort: row.get(4)?,
            },
        ))
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

fn route(
    route_kind: &str,
    route_id: &str,
    label: &str,
    model_kind: &str,
    selections: &BTreeMap<(String, String), LlmModelSelection>,
) -> LlmModelRoute {
    LlmModelRoute {
        route_kind: route_kind.to_owned(),
        route_id: route_id.to_owned(),
        label: label.to_owned(),
        model_kind: model_kind.to_owned(),
        selection: selections
            .get(&(route_kind.to_owned(), route_id.to_owned()))
            .cloned(),
    }
}

fn route_model_kind<'a>(route_kind: &str, route_id: &'a str) -> Option<&'a str> {
    match route_kind {
        "default" => DEFAULT_ROUTES
            .iter()
            .any(|(id, _)| *id == route_id)
            .then_some(route_id),
        "use_case" => USE_CASE_ROUTES
            .iter()
            .find_map(|(id, _, model_kind)| (*id == route_id).then_some(*model_kind)),
        _ => None,
    }
}

fn persist_route(transaction: &Transaction<'_>, draft: LlmModelRouteDraft) -> Result<(), AiError> {
    if let Some(selection) = draft.selection {
        let model_kind = route_model_kind(&draft.route_kind, &draft.route_id)
            .ok_or_else(|| AiError::InvalidInput("未知模型路由".to_owned()))?;
        transaction.execute(
            "insert into llm_model_routes
             (route_kind, route_id, model_kind, provider_id, model_id, reasoning_effort, updated_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(route_kind, route_id) do update set
                 model_kind = excluded.model_kind,
                 provider_id = excluded.provider_id,
                 model_id = excluded.model_id,
                 reasoning_effort = excluded.reasoning_effort,
                 updated_at = excluded.updated_at",
            params![
                draft.route_kind,
                draft.route_id,
                model_kind,
                selection.provider_id,
                selection.model_id.trim(),
                normalized_reasoning_effort(
                    &selection.provider_id,
                    selection.reasoning_effort.as_deref()
                )?
            ],
        )?;
    } else {
        transaction.execute(
            "delete from llm_model_routes where route_kind = ?1 and route_id = ?2",
            params![draft.route_kind, draft.route_id],
        )?;
    }
    Ok(())
}

fn normalized_reasoning_effort(
    provider_id: &str,
    value: Option<&str>,
) -> Result<Option<String>, AiError> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    if provider_id != "openai" {
        return Ok(None);
    }
    match value {
        Some(value) if matches!(value, "none" | "low" | "medium" | "high" | "xhigh" | "max") => {
            Ok(Some(value.to_owned()))
        }
        Some(_) => Err(AiError::InvalidInput(
            "OpenAI reasoning effort 无效".to_owned(),
        )),
        None => Ok(None),
    }
}

pub(crate) fn resolve_model_selection(
    connection: &Connection,
    task_kind: &str,
    run_override: Option<&LlmModelSelectionDraft>,
) -> Result<LlmModelSelection, AiError> {
    if let Some(selection) = run_override {
        let model_id = selection.model_id.trim();
        if model_id.is_empty() || model_id.len() > 240 {
            return Err(AiError::InvalidInput(
                "模型 ID 不能为空且不得超过 240 字符".to_owned(),
            ));
        }
        return Ok(LlmModelSelection {
            provider_id: selection.provider_id.clone(),
            model_id: model_id.to_owned(),
            reasoning_effort: normalized_reasoning_effort(
                &selection.provider_id,
                selection.reasoning_effort.as_deref(),
            )?,
        });
    }
    let routes = load_routes(connection)?;
    let model_kind = route_model_kind("use_case", task_kind).unwrap_or("text");
    routes
        .get(&("use_case".to_owned(), task_kind.to_owned()))
        .or_else(|| routes.get(&("default".to_owned(), model_kind.to_owned())))
        .or_else(|| routes.get(&("default".to_owned(), "text".to_owned())))
        .cloned()
        .ok_or_else(|| AiError::InvalidInput("请先配置文本模型或当前功能模型".to_owned()))
}

pub(crate) fn generation_settings(
    connection: &Connection,
    credentials: &JsonCredentialStore,
    provider_id: &str,
    model_id: &str,
) -> Result<AiProviderSettings, AiError> {
    let provider = get_configuration(connection, credentials)?
        .providers
        .into_iter()
        .find(|provider| provider.provider_id == provider_id)
        .ok_or_else(|| AiError::InvalidInput("未知文本生成 Provider".to_owned()))?;
    if !provider.enabled || !provider.model_kinds.iter().any(|kind| kind == "text") {
        return Err(AiError::InvalidInput(
            "所选文本生成 Provider 尚未启用".to_owned(),
        ));
    }
    Ok(AiProviderSettings {
        provider_id: provider.provider_id,
        provider_kind: "generation".to_owned(),
        enabled: provider.enabled,
        base_url: provider.base_url,
        region: String::new(),
        model_id: model_id.to_owned(),
        thinking_mode: "disabled".to_owned(),
        timeout_ms: provider.timeout_ms,
        is_cloud: true,
        credential_configured: provider.credential_configured,
        credential_mask: provider.credential_mask,
        updated_at: String::new(),
    })
}

pub(crate) fn validate_base_url(provider_id: &str, value: &str) -> Result<(), AiError> {
    let parsed = reqwest::Url::parse(value.trim())
        .map_err(|_| AiError::InvalidInput("Provider Base URL 无效".to_owned()))?;
    let host = parsed.host_str().unwrap_or_default();
    let expected_host = match provider_id {
        "deepseek" => host == "api.deepseek.com",
        "qwen" => host.ends_with(".aliyuncs.com"),
        _ => false,
    };
    if parsed.scheme() != "https"
        || !expected_host
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AiError::InvalidInput(
            "Provider Base URL 必须是目录所允许的无凭据 HTTPS 地址".to_owned(),
        ));
    }
    Ok(())
}

fn http_error(status: StatusCode) -> AiError {
    match status.as_u16() {
        401 | 403 => AiError::Provider("authentication"),
        402 => AiError::Provider("insufficient_balance"),
        429 => AiError::Provider("rate_limited"),
        500..=599 => AiError::Provider("temporary_unavailable"),
        _ => AiError::Provider("invalid_request"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_provider_level_defaults_and_capabilities() {
        let catalog = catalog().expect("valid embedded provider catalog");
        let qwen = catalog
            .providers
            .iter()
            .find(|provider| provider.id == "qwen")
            .expect("qwen provider");
        assert!(qwen.model_kinds.iter().any(|kind| kind == "embedding"));
        assert!(qwen.default_base_urls[0].contains("compatible-mode/v1"));
        assert!(
            !catalog
                .providers
                .iter()
                .any(|provider| provider.id == "qwen-embedding")
        );
    }

    #[test]
    fn provider_urls_are_restricted_to_catalog_domains() {
        assert!(validate_base_url("deepseek", "https://api.deepseek.com").is_ok());
        assert!(validate_base_url("qwen", "https://example.aliyuncs.com/v1").is_ok());
        assert!(validate_base_url("deepseek", "https://example.com").is_err());
        assert!(validate_base_url("qwen", "http://example.aliyuncs.com/v1").is_err());
    }
}
