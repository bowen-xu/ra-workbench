use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use super::error::AiError;

const CREDENTIAL_SCHEMA_VERSION: u32 = 1;
const DEVELOPMENT_CREDENTIAL_PATH_VARIABLE: &str = "RESEARCH_ASSISTANT_CREDENTIALS_PATH";

#[derive(Clone)]
pub(crate) struct JsonCredentialStore {
    path: PathBuf,
    allow_legacy_development_read: bool,
    operation_lock: Arc<Mutex<()>>,
}

pub(crate) struct ProviderSecret(String);

impl ProviderSecret {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSummary {
    pub(crate) provider_id: String,
    pub(crate) configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) masked_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderCredential {
    api_key: String,
    #[serde(flatten)]
    extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialDocument {
    schema_version: u32,
    providers: BTreeMap<String, ProviderCredential>,
    #[serde(default)]
    mcp: BTreeMap<String, ProviderCredential>,
    #[serde(flatten)]
    extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct LegacyProviderCredential {
    #[serde(rename = "api-key", alias = "api_key", alias = "apiKey")]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LegacyCredentialDocument {
    #[serde(rename = "DeepSeek")]
    deepseek: Option<LegacyProviderCredential>,
    #[serde(rename = "QWen")]
    qwen: Option<LegacyProviderCredential>,
}

impl CredentialDocument {
    fn empty() -> Self {
        Self {
            schema_version: CREDENTIAL_SCHEMA_VERSION,
            providers: BTreeMap::new(),
            mcp: BTreeMap::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn validate(&self) -> Result<(), AiError> {
        if self.schema_version != CREDENTIAL_SCHEMA_VERSION {
            return Err(AiError::CredentialInvalid);
        }
        if self
            .providers
            .keys()
            .any(|provider| !matches!(provider.as_str(), "deepseek" | "qwen" | "qwen-embedding"))
            || self
                .providers
                .values()
                .any(|credential| credential.api_key.trim().is_empty())
        {
            return Err(AiError::CredentialInvalid);
        }
        if self.mcp.keys().any(|handle| !valid_mcp_handle(handle))
            || self
                .mcp
                .values()
                .any(|credential| credential.api_key.trim().is_empty())
        {
            return Err(AiError::CredentialInvalid);
        }
        Ok(())
    }
}

impl JsonCredentialStore {
    pub(crate) fn from_app(app: &AppHandle) -> Result<Self, AiError> {
        if cfg!(feature = "development-data")
            && let Some(path) = std::env::var_os(DEVELOPMENT_CREDENTIAL_PATH_VARIABLE)
        {
            return Ok(Self::from_explicit_development_path(PathBuf::from(path)));
        }
        let path = app
            .path()
            .app_config_dir()
            .map_err(AiError::CredentialLocation)?
            .join("ai-credentials.json");
        Ok(Self::from_path(path))
    }

    pub(crate) fn from_path(path: PathBuf) -> Self {
        Self {
            path,
            allow_legacy_development_read: false,
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn from_explicit_development_path(path: PathBuf) -> Self {
        Self {
            path,
            allow_legacy_development_read: true,
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn summary(&self, provider_id: &str) -> Result<CredentialSummary, AiError> {
        validate_provider_id(provider_id)?;
        let _guard = self.operation_lock.lock().map_err(|_| {
            AiError::CredentialIo(std::io::Error::other("credential lock unavailable"))
        })?;
        let document = self.read_document(true)?;
        let masked_value = credential_for(&document, provider_id)
            .map(|credential| mask_secret(&credential.api_key));
        Ok(CredentialSummary {
            provider_id: provider_id.to_owned(),
            configured: masked_value.is_some(),
            masked_value,
        })
    }

    pub(crate) fn resolve(&self, provider_id: &str) -> Result<ProviderSecret, AiError> {
        validate_provider_id(provider_id)?;
        let _guard = self.operation_lock.lock().map_err(|_| {
            AiError::CredentialIo(std::io::Error::other("credential lock unavailable"))
        })?;
        credential_for(&self.read_document(true)?, provider_id)
            .map(|credential| ProviderSecret(credential.api_key.clone()))
            .ok_or(AiError::CredentialMissing)
    }

    /// Resolve an opaque MCP handle from the same private credential store.
    /// The handle namespace is separate from provider IDs so MCP credentials
    /// cannot accidentally be used as model-provider credentials.
    pub(crate) fn resolve_mcp(&self, handle: &str) -> Result<ProviderSecret, AiError> {
        if !valid_mcp_handle(handle) {
            return Err(AiError::InvalidInput("MCP 凭据句柄无效".to_owned()));
        }
        let _guard = self.operation_lock.lock().map_err(|_| {
            AiError::CredentialIo(std::io::Error::other("credential lock unavailable"))
        })?;
        self.read_document(true)?
            .mcp
            .get(handle)
            .map(|credential| ProviderSecret(credential.api_key.clone()))
            .ok_or(AiError::CredentialMissing)
    }

    pub(crate) fn set_mcp(&self, handle: &str, secret: Option<&str>) -> Result<(), AiError> {
        if !valid_mcp_handle(handle)
            || secret.is_some_and(|secret| secret.trim().len() < 8 || secret.len() > 4096)
        {
            return Err(AiError::InvalidInput("MCP 凭据无效".to_owned()));
        }
        let _guard = self.operation_lock.lock().map_err(|_| {
            AiError::CredentialIo(std::io::Error::other("credential lock unavailable"))
        })?;
        let mut document = self.read_document(false)?;
        if let Some(secret) = secret {
            document.mcp.insert(
                handle.to_owned(),
                ProviderCredential {
                    api_key: secret.to_owned(),
                    extensions: BTreeMap::new(),
                },
            );
        } else {
            document.mcp.remove(handle);
        }
        self.write_document(&document)
    }

    #[cfg(test)]
    pub(crate) fn set_mcp_for_test(&self, handle: &str, secret: &str) -> Result<(), AiError> {
        self.set_mcp(handle, Some(secret))
    }

    pub(crate) fn set(
        &self,
        provider_id: &str,
        api_key: &str,
    ) -> Result<CredentialSummary, AiError> {
        validate_provider_id(provider_id)?;
        let api_key = api_key.trim();
        if api_key.len() < 8 || api_key.len() > 4096 {
            return Err(AiError::InvalidInput(
                "API key 长度无效；现有凭据未被修改".to_owned(),
            ));
        }
        let _guard = self.operation_lock.lock().map_err(|_| {
            AiError::CredentialIo(std::io::Error::other("credential lock unavailable"))
        })?;
        let mut document = self.read_document(false)?;
        document
            .providers
            .entry(provider_id.to_owned())
            .and_modify(|credential| {
                credential.api_key.clear();
                credential.api_key.push_str(api_key);
            })
            .or_insert_with(|| ProviderCredential {
                api_key: api_key.to_owned(),
                extensions: BTreeMap::new(),
            });
        if provider_id == "qwen" {
            document.providers.remove("qwen-embedding");
        }
        self.write_document(&document)?;
        Ok(CredentialSummary {
            provider_id: provider_id.to_owned(),
            configured: true,
            masked_value: Some(mask_secret(api_key)),
        })
    }

    pub(crate) fn delete(&self, provider_id: &str) -> Result<CredentialSummary, AiError> {
        validate_provider_id(provider_id)?;
        let _guard = self.operation_lock.lock().map_err(|_| {
            AiError::CredentialIo(std::io::Error::other("credential lock unavailable"))
        })?;
        let mut document = self.read_document(false)?;
        document.providers.remove(provider_id);
        if provider_id == "qwen" {
            document.providers.remove("qwen-embedding");
        }
        self.write_document(&document)?;
        Ok(CredentialSummary {
            provider_id: provider_id.to_owned(),
            configured: false,
            masked_value: None,
        })
    }

    fn read_document(&self, allow_legacy: bool) -> Result<CredentialDocument, AiError> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                    return Err(AiError::CredentialFileType);
                }
                validate_permissions(&metadata)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CredentialDocument::empty());
            }
            Err(error) => return Err(AiError::CredentialIo(error)),
        }
        let bytes = fs::read(&self.path).map_err(AiError::CredentialIo)?;
        if bytes.len() > 1024 * 1024 {
            return Err(AiError::CredentialInvalid);
        }
        if let Ok(document) = serde_json::from_slice::<CredentialDocument>(&bytes) {
            document.validate()?;
            return Ok(document);
        }
        if allow_legacy && self.allow_legacy_development_read {
            return import_legacy_development_document(&bytes);
        }
        Err(AiError::CredentialInvalid)
    }

    fn write_document(&self, document: &CredentialDocument) -> Result<(), AiError> {
        document.validate()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AiError::InvalidInput("凭据文件必须位于一个明确目录中".to_owned()))?;
        create_private_directory(parent)?;
        let file_name = self
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AiError::InvalidInput("凭据文件名无效".to_owned()))?;
        let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
        let result = write_temporary(&temporary, document)
            .and_then(|()| fs::rename(&temporary, &self.path).map_err(AiError::CredentialIo))
            .and_then(|()| sync_directory(parent));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn import_legacy_development_document(bytes: &[u8]) -> Result<CredentialDocument, AiError> {
    let legacy: LegacyCredentialDocument =
        serde_json::from_slice(bytes).map_err(|_| AiError::CredentialInvalid)?;
    if legacy.deepseek.is_none() && legacy.qwen.is_none() {
        return Err(AiError::CredentialInvalid);
    }
    let mut document = CredentialDocument::empty();
    for (provider_id, credential) in [("deepseek", legacy.deepseek), ("qwen", legacy.qwen)] {
        if let Some(api_key) = credential.and_then(|value| value.api_key) {
            let api_key = api_key.trim();
            if api_key.is_empty() {
                return Err(AiError::CredentialInvalid);
            }
            document.providers.insert(
                provider_id.to_owned(),
                ProviderCredential {
                    api_key: api_key.to_owned(),
                    extensions: BTreeMap::new(),
                },
            );
        }
    }
    document.validate()?;
    Ok(document)
}

fn validate_provider_id(provider_id: &str) -> Result<(), AiError> {
    if matches!(provider_id, "deepseek" | "qwen" | "qwen-embedding") {
        Ok(())
    } else {
        Err(AiError::InvalidInput("不支持的凭据 provider".to_owned()))
    }
}

fn valid_mcp_handle(handle: &str) -> bool {
    handle.starts_with("mcp:")
        && handle.len() > 4
        && handle.len() <= 256
        && !handle.chars().any(char::is_control)
}

fn credential_for<'a>(
    document: &'a CredentialDocument,
    provider_id: &str,
) -> Option<&'a ProviderCredential> {
    document
        .providers
        .get(provider_id)
        .or_else(|| match provider_id {
            "qwen" => document.providers.get("qwen-embedding"),
            "qwen-embedding" => document.providers.get("qwen"),
            _ => None,
        })
}

fn mask_secret(value: &str) -> String {
    let suffix: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("••••{suffix}")
}

fn create_private_directory(path: &Path) -> Result<(), AiError> {
    fs::create_dir_all(path).map_err(AiError::CredentialIo)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(AiError::CredentialIo)?;
    Ok(())
}

fn validate_permissions(metadata: &fs::Metadata) -> Result<(), AiError> {
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(AiError::CredentialPermissions);
    }
    Ok(())
}

fn write_temporary(path: &Path, document: &CredentialDocument) -> Result<(), AiError> {
    let bytes = serde_json::to_vec_pretty(document).map_err(|_| AiError::CredentialInvalid)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).map_err(AiError::CredentialIo)?;
    file.write_all(&bytes).map_err(AiError::CredentialIo)?;
    file.write_all(b"\n").map_err(AiError::CredentialIo)?;
    file.sync_all().map_err(AiError::CredentialIo)
}

fn sync_directory(path: &Path) -> Result<(), AiError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(AiError::CredentialIo)
}

#[cfg(test)]
#[path = "credential_tests.rs"]
mod tests;
