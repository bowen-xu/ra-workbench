use rusqlite::{Connection, params};

use super::{
    credential::JsonCredentialStore,
    error::AiError,
    model::{AiProviderOverview, AiProviderSettings, AiProviderSettingsDraft},
    provider::capabilities_for,
};

pub(crate) fn list_provider_overviews(
    connection: &Connection,
    credentials: &JsonCredentialStore,
) -> Result<Vec<AiProviderOverview>, AiError> {
    let mut statement = connection.prepare(
        "select provider_id, provider_kind, enabled, base_url, region, model_id,
                thinking_mode, timeout_ms, updated_at
         from ai_provider_settings
         where provider_id != 'openai'
         order by
             case provider_id when 'fixture-generation' then 0 when 'deepseek' then 1 else 2 end",
    )?;
    let rows = statement.query_map([], settings_from_row)?;
    rows.map(|row| {
        let mut settings = row?;
        let capabilities = capabilities_for(&settings.provider_id)
            .ok_or_else(|| AiError::InvalidInput("未知 AI provider 设置".to_owned()))?;
        settings.is_cloud = capabilities.is_cloud;
        if capabilities.is_cloud {
            let summary = credentials.summary(&settings.provider_id)?;
            settings.credential_configured = summary.configured;
            settings.credential_mask = summary.masked_value;
        } else {
            settings.credential_configured = true;
        }
        Ok(AiProviderOverview {
            settings,
            capabilities,
        })
    })
    .collect()
}

pub(crate) fn get_provider_settings(
    connection: &Connection,
    credentials: &JsonCredentialStore,
    provider_id: &str,
) -> Result<AiProviderSettings, AiError> {
    list_provider_overviews(connection, credentials)?
        .into_iter()
        .find(|overview| overview.settings.provider_id == provider_id)
        .map(|overview| overview.settings)
        .ok_or_else(|| AiError::InvalidInput("未知 AI provider".to_owned()))
}

pub(crate) fn save_provider_settings(
    connection: &Connection,
    credentials: &JsonCredentialStore,
    draft: AiProviderSettingsDraft,
) -> Result<AiProviderSettings, AiError> {
    validate_settings(&draft)?;
    let existing_kind: String = connection
        .query_row(
            "select provider_kind from ai_provider_settings where provider_id = ?1",
            [&draft.provider_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AiError::InvalidInput("未知 AI provider".to_owned())
            }
            other => AiError::Database(other),
        })?;
    if draft.enabled && draft.provider_id != "fixture-generation" {
        let summary = credentials.summary(&draft.provider_id)?;
        if !summary.configured {
            return Err(AiError::CredentialMissing);
        }
    }
    connection.execute(
        "update ai_provider_settings
         set enabled = ?2, base_url = ?3, region = ?4, model_id = ?5,
             thinking_mode = ?6, timeout_ms = ?7,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where provider_id = ?1",
        params![
            draft.provider_id,
            draft.enabled,
            draft.base_url.trim(),
            draft.region.trim(),
            draft.model_id.trim(),
            draft.thinking_mode,
            draft.timeout_ms
        ],
    )?;
    let settings = get_provider_settings(connection, credentials, &draft.provider_id)?;
    if settings.provider_kind != existing_kind {
        return Err(AiError::InvalidInput(
            "provider 类型不能通过设置修改".to_owned(),
        ));
    }
    Ok(settings)
}

fn settings_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiProviderSettings> {
    Ok(AiProviderSettings {
        provider_id: row.get(0)?,
        provider_kind: row.get(1)?,
        enabled: row.get(2)?,
        base_url: row.get(3)?,
        region: row.get(4)?,
        model_id: row.get(5)?,
        thinking_mode: row.get(6)?,
        timeout_ms: row.get(7)?,
        is_cloud: false,
        credential_configured: false,
        credential_mask: None,
        updated_at: row.get(8)?,
    })
}

fn validate_settings(draft: &AiProviderSettingsDraft) -> Result<(), AiError> {
    if !(1_000..=300_000).contains(&draft.timeout_ms) {
        return Err(AiError::InvalidInput(
            "provider timeout 必须在 1–300 秒之间".to_owned(),
        ));
    }
    if !matches!(draft.thinking_mode.as_str(), "disabled" | "enabled") {
        return Err(AiError::InvalidInput("无效的 thinking mode".to_owned()));
    }
    if draft.model_id.trim().len() > 200 || draft.region.trim().len() > 100 {
        return Err(AiError::InvalidInput("provider 设置字段过长".to_owned()));
    }
    match draft.provider_id.as_str() {
        "fixture-generation" => {
            if draft.base_url.trim().is_empty()
                && draft.region.trim() == "local"
                && draft.model_id.trim() == "fixture-v1"
                && draft.thinking_mode == "disabled"
            {
                Ok(())
            } else {
                Err(AiError::InvalidInput(
                    "fixture provider 的固定身份不能修改".to_owned(),
                ))
            }
        }
        "deepseek" => {
            if !matches!(
                draft.base_url.trim_end_matches('/'),
                "https://api.deepseek.com" | "https://api.deepseek.com/v1"
            ) {
                return Err(AiError::InvalidInput(
                    "DeepSeek base URL 必须使用受支持的官方 HTTPS 地址".to_owned(),
                ));
            }
            if matches!(draft.model_id.trim(), "deepseek-chat" | "deepseek-reasoner") {
                return Err(AiError::InvalidInput(
                    "该 DeepSeek 模型别名已停用；请从官方模型列表选择当前模型 ID".to_owned(),
                ));
            }
            require_model_when_enabled(draft)
        }
        "qwen-embedding" => {
            validate_qwen_url(&draft.base_url)?;
            if draft.enabled && draft.base_url.trim().is_empty() {
                return Err(AiError::InvalidInput(
                    "启用 QWen 前必须填写受支持的阿里云 endpoint".to_owned(),
                ));
            }
            if draft.thinking_mode != "disabled" {
                return Err(AiError::InvalidInput(
                    "embedding provider 不支持 thinking mode".to_owned(),
                ));
            }
            require_model_when_enabled(draft)
        }
        _ => Err(AiError::InvalidInput("未知 AI provider".to_owned())),
    }
}

fn require_model_when_enabled(draft: &AiProviderSettingsDraft) -> Result<(), AiError> {
    if draft.enabled && draft.model_id.trim().is_empty() {
        Err(AiError::InvalidInput(
            "启用 provider 前必须选择明确模型".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn validate_qwen_url(value: &str) -> Result<(), AiError> {
    if value.trim().is_empty() {
        return Ok(());
    }
    let parsed = reqwest::Url::parse(value.trim())
        .map_err(|_| AiError::InvalidInput("QWen base URL 无效".to_owned()))?;
    let host = parsed.host_str().unwrap_or_default();
    if parsed.scheme() != "https"
        || !host.ends_with(".aliyuncs.com")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AiError::InvalidInput(
            "QWen base URL 必须是无凭据、查询或片段的阿里云 HTTPS 地址".to_owned(),
        ));
    }
    Ok(())
}
