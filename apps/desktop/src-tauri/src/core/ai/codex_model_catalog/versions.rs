//! Discovery changes only the models GET's client_version, never the sidecar.
use super::{CLIENT_VERSION, CodexModelCatalog, cache};
use crate::ai::error::AiError;
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

const RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const MAX_RELEASE_BYTES: usize = 1024 * 1024;
const MAX_VERSIONS: usize = 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodexCatalogVersions {
    pub runtime_version: String,
    pub selected_version: String,
    pub available_versions: Vec<String>,
}
impl Default for CodexCatalogVersions {
    fn default() -> Self {
        Self {
            runtime_version: CLIENT_VERSION.into(),
            selected_version: CLIENT_VERSION.into(),
            available_versions: vec![CLIENT_VERSION.into()],
        }
    }
}
fn version_key(value: &str) -> Option<[u32; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut result = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty()
            || part.len() > 9
            || !part.bytes().all(|b| b.is_ascii_digit())
            || part.len() > 1 && part.starts_with('0')
        {
            return None;
        }
        result[index] = part.parse().ok()?;
    }
    Some(result)
}
pub(crate) fn is_version(value: &str) -> bool {
    version_key(value).is_some()
}
fn error(code: &'static str, cause: impl ToString) -> AiError {
    AiError::CodexVersion {
        code,
        cause: cause.to_string(),
    }
}
fn read(path: &Path) -> Result<CodexCatalogVersions, AiError> {
    let bytes = match cache::read_bounded(path) {
        Ok(bytes) => bytes,
        Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CodexCatalogVersions::default());
        }
        Err(reason) => return Err(error("codex_version_settings", reason)),
    };
    let mut value: CodexCatalogVersions = serde_json::from_slice(&bytes)
        .map_err(|_| error("codex_version_settings", "版本设置格式无效"))?;
    if !is_version(&value.selected_version)
        || value.available_versions.len() > MAX_VERSIONS
        || value.available_versions.is_empty()
        || value
            .available_versions
            .iter()
            .any(|item| !is_version(item))
        || !value.available_versions.contains(&value.selected_version)
    {
        return Err(error("codex_version_settings", "版本设置内容无效"));
    }
    // This reports the bundled adapter, never a persisted or selected identity.
    value.runtime_version = CLIENT_VERSION.into();
    merge(&mut value, CLIENT_VERSION.into());
    Ok(value)
}
fn merge(value: &mut CodexCatalogVersions, latest: String) {
    value.available_versions.extend([
        latest,
        value.selected_version.clone(),
        CLIENT_VERSION.into(),
    ]);
    value
        .available_versions
        .sort_by_key(|item| std::cmp::Reverse(version_key(item)));
    value.available_versions.dedup();
    while value.available_versions.len() > MAX_VERSIONS {
        let index = value
            .available_versions
            .iter()
            .rposition(|item| item != &value.selected_version && item != CLIENT_VERSION)
            .unwrap();
        value.available_versions.remove(index);
    }
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}
fn decode_release(bytes: &[u8]) -> Result<String, AiError> {
    let release: Release = serde_json::from_slice(bytes)
        .map_err(|_| error("codex_version_response", "GitHub 版本数据格式无效"))?;
    let version = release
        .tag_name
        .strip_prefix("rust-v")
        .filter(|value| is_version(value));
    if release.draft || release.prerelease || version.is_none() {
        return Err(error(
            "codex_version_response",
            "未返回有效的 Codex CLI 正式版本",
        ));
    }
    Ok(version.unwrap().into())
}
async fn latest() -> Result<String, AiError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|reason| error("codex_version_network", reason))?;
    // No ChatGPT credential, prompt, account identifier or installation action.
    let mut response = client
        .get(RELEASE_URL)
        .header("User-Agent", "Research-Assistant")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|reason| error("codex_version_network", reason))?;
    if !response.status().is_success() {
        return Err(error(
            "codex_version_http",
            format!(
                "获取 Codex CLI 版本失败：HTTP {}",
                response.status().as_u16()
            ),
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RELEASE_BYTES as u64)
    {
        return Err(error("codex_version_response", "版本数据过大"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|reason| error("codex_version_network", reason))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RELEASE_BYTES {
            return Err(error("codex_version_response", "版本数据过大"));
        }
        bytes.extend_from_slice(&chunk);
    }
    decode_release(&bytes)
}
impl CodexModelCatalog {
    pub(super) async fn read_versions(&self) -> Result<CodexCatalogVersions, AiError> {
        let path = self.version_settings.clone();
        tokio::task::spawn_blocking(move || read(&path))
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?
    }
    pub(crate) async fn versions(&self, refresh: bool) -> Result<CodexCatalogVersions, AiError> {
        let _lock = self.refresh.lock().await;
        let mut value = self.read_versions().await?;
        if refresh {
            merge(&mut value, latest().await?);
            self.write_versions(&value).await?;
        }
        Ok(value)
    }
    pub(crate) async fn select_version(
        &self,
        version: String,
    ) -> Result<CodexCatalogVersions, AiError> {
        let _lock = self.refresh.lock().await;
        let mut value = self.read_versions().await?;
        if !value.available_versions.contains(&version) {
            return Err(AiError::InvalidInput(
                "请选择已获取的 Codex CLI 版本".into(),
            ));
        }
        value.selected_version = version;
        self.write_versions(&value).await?;
        Ok(value)
    }
    async fn write_versions(&self, value: &CodexCatalogVersions) -> Result<(), AiError> {
        let path = self.version_settings.clone();
        let value = value.clone();
        tokio::task::spawn_blocking(move || {
            cache::write_json(&path, &value)
                .map_err(|reason| error("codex_version_settings", reason))
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_discovery_preserves_selection_and_runtime_and_rejects_non_stable_data() {
        let latest =
            decode_release(br#"{"tag_name":"rust-v0.200.0","draft":false,"prerelease":false}"#)
                .unwrap();
        let mut value = CodexCatalogVersions::default();
        merge(&mut value, latest);
        assert_eq!(value.selected_version, CLIENT_VERSION);
        assert_eq!(value.runtime_version, CLIENT_VERSION);
        assert_eq!(value.available_versions, vec!["0.200.0", CLIENT_VERSION]);
        for tag in [
            "rust-v0.200.0-alpha.1",
            "v0.200.0",
            "rust-v00.2.0",
            "rust-v1.2",
        ] {
            assert!(
                decode_release(
                    serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false})
                        .to_string()
                        .as_bytes()
                )
                .is_err()
            );
        }
        assert!(
            decode_release(br#"{"tag_name":"rust-v0.200.0","draft":false,"prerelease":true}"#)
                .is_err()
        );
    }
    #[test]
    fn preferences_reopen_and_bound_history_without_losing_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("versions.json");
        let mut value = read(&path).unwrap();
        merge(&mut value, "0.160.0".into());
        value.selected_version = "0.160.0".into();
        for patch in 0..30 {
            merge(&mut value, format!("1.0.{patch}"));
        }
        assert_eq!(value.available_versions.len(), MAX_VERSIONS);
        cache::write_json(&path, &value).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.selected_version, "0.160.0");
        assert_eq!(loaded.runtime_version, CLIENT_VERSION);
        std::fs::write(&path, "broken").unwrap();
        assert!(read(&path).is_err());
    }
}
