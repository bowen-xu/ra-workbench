use std::{fs, path::Path};

use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::error::DoclingError;

pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const PROTOCOL_VERSION: u32 = research_docling_runtime::PROTOCOL_VERSION;
pub(crate) const STATUS_PATH: &str = "/local-service/v1/status";
pub(crate) const LEASE_HEADER: &str = "X-Docling-Lease-Id";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryRecord {
    pub(crate) schema_version: u32,
    pub(crate) protocol_version: u32,
    pub(crate) service_instance_id: String,
    pub(crate) state: String,
    pub(crate) http_base_url: String,
    pub(crate) websocket_url: String,
    pub(crate) pid: u32,
    pub(crate) started_at: String,
    pub(crate) wrapper_version: String,
    pub(crate) docling_serve_version: String,
    pub(crate) heartbeat_interval_ms: u64,
    pub(crate) lease_timeout_ms: u64,
    pub(crate) idle_shutdown_ms: u64,
    pub(crate) model_identity: Option<String>,
    pub(crate) precision: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonStatus {
    #[serde(default)]
    pub(crate) service_revision: u32,
    pub(crate) schema_version: u32,
    pub(crate) protocol_version: u32,
    pub(crate) service_instance_id: String,
    pub(crate) state: String,
    pub(crate) model_identity: Option<String>,
    pub(crate) precision: Option<String>,
    pub(crate) active_client_count: Option<u32>,
    pub(crate) active_request_count: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LeaseHello<'a> {
    pub(crate) r#type: &'static str,
    pub(crate) protocol_version: u32,
    pub(crate) client_id: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LeaseGranted {
    pub(crate) r#type: String,
    pub(crate) lease_id: String,
    pub(crate) service_instance_id: String,
    pub(crate) heartbeat_interval_ms: u64,
    pub(crate) lease_timeout_ms: u64,
    #[serde(default = "connected_client_count_default")]
    pub(crate) active_client_count: u32,
    pub(crate) active_request_count: Option<u32>,
}

fn connected_client_count_default() -> u32 {
    1
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LeaseHeartbeat<'a> {
    pub(crate) r#type: &'static str,
    pub(crate) lease_id: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LeaseHeartbeatAck {
    pub(crate) r#type: String,
    pub(crate) lease_id: String,
    pub(crate) active_client_count: Option<u32>,
    pub(crate) active_request_count: Option<u32>,
}

pub(crate) fn read_registry(path: &Path) -> Result<Option<RegistryRecord>, DoclingError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| DoclingError::InvalidRegistry(error.to_string()))
}

pub(crate) fn validate_registry(record: &RegistryRecord) -> Result<(), DoclingError> {
    if record.schema_version != SCHEMA_VERSION || record.protocol_version != PROTOCOL_VERSION {
        return Err(DoclingError::Incompatible(format!(
            "registry schema/protocol 为 {}/{}，当前仅支持 {}/{}",
            record.schema_version, record.protocol_version, SCHEMA_VERSION, PROTOCOL_VERSION
        )));
    }
    if record.state != "ready" {
        return Err(DoclingError::Unresponsive(format!(
            "Docling daemon 当前状态为 {}",
            record.state
        )));
    }
    validate_loopback_url(&record.http_base_url, "http")?;
    validate_loopback_url(&record.websocket_url, "ws")?;
    if record.service_instance_id.trim().is_empty()
        || record.heartbeat_interval_ms == 0
        || record.lease_timeout_ms <= record.heartbeat_interval_ms
    {
        return Err(DoclingError::InvalidRegistry(
            "registry 缺少有效实例或 heartbeat 参数".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_status(
    record: &RegistryRecord,
    status: &DaemonStatus,
) -> Result<(), DoclingError> {
    if status.service_revision != research_docling_runtime::SERVICE_REVISION {
        return Err(DoclingError::Incompatible(format!(
            "Docling 服务能力修订不兼容（当前 {}，需要 {}）；请重新构建 Docling 程序后手动连接",
            status.service_revision,
            research_docling_runtime::SERVICE_REVISION
        )));
    }
    if status.schema_version != SCHEMA_VERSION || status.protocol_version != PROTOCOL_VERSION {
        return Err(DoclingError::Incompatible(
            "Docling daemon 协议版本不兼容".to_owned(),
        ));
    }
    if status.service_instance_id != record.service_instance_id || status.state != "ready" {
        return Err(DoclingError::Unresponsive(
            "Docling daemon 身份或状态与 registry 不一致".to_owned(),
        ));
    }
    Ok(())
}

fn validate_loopback_url(value: &str, expected_scheme: &str) -> Result<(), DoclingError> {
    let url =
        Url::parse(value).map_err(|error| DoclingError::InvalidRegistry(error.to_string()))?;
    if url.scheme() != expected_scheme
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
    {
        return Err(DoclingError::InvalidRegistry(format!(
            "服务地址必须使用 {expected_scheme} 并绑定 127.0.0.1"
        )));
    }
    Ok(())
}

pub(crate) fn validate_model_identity(
    record: &RegistryRecord,
    status: &DaemonStatus,
    expected: &str,
) -> Result<(), DoclingError> {
    if record.model_identity.as_deref() != Some(expected)
        || status.model_identity.as_deref() != Some(expected)
        || record.precision.as_deref() != Some("fp32")
        || status.precision.as_deref() != Some("fp32")
        || record.docling_serve_version != research_docling_runtime::ENGINE_VERSION
    {
        return Err(DoclingError::Incompatible(
            "Docling 引擎、模型身份或精度不匹配".into(),
        ));
    }
    Ok(())
}
