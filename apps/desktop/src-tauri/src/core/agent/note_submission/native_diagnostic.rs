//! Safe native/data causes bound to the original Core Note submission receipt.
use super::{Error, SourceReceipt};
use crate::{
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::{
        literature_trace::data::TraceDataError,
        native::{NativePreviewError, NativeTerminationDiagnostic},
    },
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error(
    "{code}: {message} (conversationId={conversation_id}; requestId={request_id}; pageId={page_id})"
)]
pub struct NativeDiagnostic {
    pub code: String,
    pub message: String,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub page_id: Uuid,
    pub plugin_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_error: Option<NativeTerminationDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqlite_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqlite_extended_code: Option<i32>,
}

impl NativeDiagnostic {
    fn new(receipt: SourceReceipt, key: &str, code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            conversation_id: receipt.conversation_id,
            request_id: receipt.request_id,
            page_id: receipt.resource.id(),
            plugin_id: "literature_library".into(),
            package_key: Some(key.into()),
            instance_id: None,
            native_request_id: None,
            exit_code: None,
            signal: None,
            cleanup_error: None,
            termination_error: None,
            resource_id: None,
            resource_version: None,
            phase: None,
            safe_cause: None,
            sqlite_code: None,
            sqlite_extended_code: None,
        }
    }
}

pub(super) fn host(receipt: SourceReceipt, key: &str, cause: NativePreviewError) -> Error {
    let mut diagnostic = NativeDiagnostic::new(receipt, key, &cause.code, &cause.message);
    diagnostic.plugin_id = cause.plugin_id.into();
    diagnostic.package_key = cause.package_key.or(diagnostic.package_key);
    diagnostic.instance_id = cause.instance_id;
    diagnostic.native_request_id = cause.request_id;
    diagnostic.exit_code = cause.exit_code;
    diagnostic.signal = cause.signal;
    diagnostic.cleanup_error = cause.cleanup_error;
    diagnostic.termination_error = cause.termination_error;
    Error::Native(Box::new(diagnostic))
}

fn data(receipt: SourceReceipt, key: &str, cause: PluginDataError) -> Error {
    let mut diagnostic = NativeDiagnostic::new(receipt, key, cause.code, cause.message);
    diagnostic.package_key = Some(cause.package_key);
    diagnostic.resource_id = Some(cause.resource_id);
    diagnostic.resource_version = Some(cause.resource_version);
    diagnostic.phase = Some(cause.phase.into());
    diagnostic.safe_cause = Some(cause.safe_cause);
    diagnostic.sqlite_code = cause.sqlite_code;
    diagnostic.sqlite_extended_code = cause.sqlite_extended_code;
    Error::Native(Box::new(diagnostic))
}

pub(super) fn consumption(receipt: SourceReceipt, key: &str, cause: TraceDataError) -> Error {
    match cause {
        TraceDataError::Data(cause) => data(receipt, key, *cause),
        TraceDataError::Rejected(_) => Error::Conflict,
        TraceDataError::InvalidSource => Error::InvalidInput,
        TraceDataError::InvalidObservation => Error::Native(Box::new(NativeDiagnostic::new(
            receipt,
            key,
            "plugin_data_result_invalid",
            "检索上下文的数据形状与已接受契约不符；本次未提交。",
        ))),
        TraceDataError::InvalidDecision => Error::Native(Box::new(NativeDiagnostic::new(
            receipt,
            key,
            "plugin_host_protocol_invalid",
            "文献宿主的检索决定与固定契约不符；本次未提交。",
        ))),
    }
}

#[cfg(test)]
#[path = "native_diagnostic/tests.rs"]
mod tests;
