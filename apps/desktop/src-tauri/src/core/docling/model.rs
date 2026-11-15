use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingSettings {
    pub enabled: bool,
    pub model_manifest_url: String,
    pub default_port: u16,
    pub fallback_start_port: u16,
    pub fallback_end_port: u16,
    pub idle_shutdown_ms: u64,
    pub request_timeout_ms: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingSettingsDraft {
    pub enabled: bool,
    pub model_manifest_url: String,
    pub default_port: u16,
    pub fallback_start_port: u16,
    pub fallback_end_port: u16,
    pub idle_shutdown_ms: u64,
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingServiceStatus {
    pub state: String,
    pub base_url: Option<String>,
    pub port: Option<u16>,
    pub pid: Option<u32>,
    pub instance_id: Option<String>,
    pub last_error: Option<String>,
    pub registry_path: Option<String>,
    pub active_client_count: Option<u32>,
    pub active_request_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingPingResult {
    pub status: DoclingServiceStatus,
    pub latency_ms: u64,
}

impl DoclingServiceStatus {
    pub(crate) fn unavailable(message: &str) -> Self {
        Self {
            state: "unavailable".to_owned(),
            base_url: None,
            port: None,
            pid: None,
            instance_id: None,
            last_error: Some(message.to_owned()),
            registry_path: None,
            active_client_count: None,
            active_request_count: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingParseRunSummary {
    pub id: String,
    pub item_id: String,
    pub attachment_id: String,
    pub content_sha256: String,
    pub service_base_url: String,
    pub docling_status: String,
    pub parse_status: String,
    pub error_message: Option<String>,
    pub processing_time_seconds: Option<f64>,
    pub created_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingParseRunDetail {
    #[serde(flatten)]
    pub summary: DoclingParseRunSummary,
    pub json_content: Option<Value>,
    pub raw_response: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DoclingOverlayRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DoclingOverlayRegion {
    pub page_index: u32,
    pub rect: DoclingOverlayRect,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DoclingOverlayItem {
    pub id: String,
    pub kind: String,
    pub label: Option<String>,
    pub text_preview: Option<String>,
    pub regions: Vec<DoclingOverlayRegion>,
    pub raw_ref: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DoclingPageSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DoclingPageOverlay {
    pub attachment_id: String,
    pub parse_run_id: String,
    pub page_index: u32,
    pub page_size: Option<DoclingPageSize>,
    pub items: Vec<DoclingOverlayItem>,
}
