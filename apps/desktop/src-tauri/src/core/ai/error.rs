use serde::Serialize;
use thiserror::Error;

use crate::storage::StorageError;

#[derive(Debug, Error)]
pub(crate) enum AiError {
    #[error(transparent)]
    GenerationStorageUnknown(Box<super::generation_storage_failure::GenerationStorageUnknown>),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("AI 运行数据保存失败")]
    Database(#[from] rusqlite::Error),
    #[error("AI 运行数据保存失败")]
    PluginDatabase(#[source] Box<crate::data_interface::plugin_resources::PluginDataError>),
    #[error("{failure}")]
    NativeWorkflow {
        code: &'static str,
        failure: Box<crate::runtime_plugins::native::NativeHostError>,
    },
    #[error(
        "AI 运行创建结果尚未确认，请重启应用后核对原运行 {run_id}；阶段：{phase}；原因：{safe_cause}；SQLite：{sqlite_code:?} / {sqlite_extended_code:?}；原错误码：{prior_code:?}"
    )]
    RunCreationUnknown {
        run_id: uuid::Uuid,
        phase: &'static str,
        safe_cause: &'static str,
        prior_code: Option<&'static str>,
        sqlite_code: Option<String>,
        sqlite_extended_code: Option<i32>,
    },
    #[error("AI 运行 {run_id} 不再处于可结束状态，请核对已有终态")]
    RunTerminalConflict { run_id: String },
    #[error(
        "AI 运行 {run_id} 的终态结果尚未确认，请重启应用后核对原运行；阶段：{phase}；原因：{safe_cause}；SQLite：{sqlite_code:?} / {sqlite_extended_code:?}；原错误码：{prior_code:?}"
    )]
    RunTerminalUnknown {
        run_id: String,
        phase: &'static str,
        safe_cause: &'static str,
        prior_code: Option<&'static str>,
        sqlite_code: Option<String>,
        sqlite_extended_code: Option<i32>,
    },
    #[error(transparent)]
    Knowledge(#[from] crate::ai::knowledge_contracts::KnowledgeError),
    #[error("当前构建未提供所需能力：{0}")]
    CapabilityUnavailable(&'static str),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{path}: {message}")]
    Validation { path: String, message: String },
    #[error("没有配置该 provider 的本地凭据")]
    CredentialMissing,
    #[error("本地凭据文件格式无效；原文件未被修改")]
    CredentialInvalid,
    #[error("本地凭据文件权限不安全；请限制为仅当前用户可读写")]
    CredentialPermissions,
    #[error("本地凭据路径不是普通文件")]
    CredentialFileType,
    #[error("本地凭据文件操作失败")]
    CredentialIo(#[source] std::io::Error),
    #[error("无法定位本地凭据目录")]
    CredentialLocation(#[source] tauri::Error),
    #[error("provider 暂时不可用")]
    TemporaryUnavailable,
    #[error("provider 请求超时")]
    Timeout,
    #[error("AI 运行已取消")]
    Canceled,
    #[error("provider 请求失败")]
    Provider(&'static str),
    #[error("工具执行结果尚未确认：{0}")]
    ToolOutcomeDiagnostic(Box<research_agent_core::runner::ToolOutcomeDiagnostic>),
    #[error("{cause}")]
    CodexVersion { code: &'static str, cause: String },
    #[error("{cause}")]
    Model {
        code: &'static str,
        cause: research_agent_core::model::ModelError,
    },
}

impl AiError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::GenerationStorageUnknown(_) => "data_operation_outcome_unknown",
            Self::RunCreationUnknown { .. } => "data_operation_outcome_unknown",
            Self::NativeWorkflow { code, .. } => code,
            Self::RunTerminalConflict { .. } => "ai_run_terminal_conflict",
            Self::RunTerminalUnknown { .. } => "ai_run_terminal_outcome_unknown",
            Self::Storage(_)
            | Self::Database(_)
            | Self::PluginDatabase(_)
            | Self::Knowledge(crate::ai::knowledge_contracts::KnowledgeError::Storage(_))
            | Self::Knowledge(crate::ai::knowledge_contracts::KnowledgeError::Database(_))
            | Self::Knowledge(crate::ai::knowledge_contracts::KnowledgeError::SemanticIndex(_)) => {
                "storage_error"
            }
            Self::Knowledge(
                crate::ai::knowledge_contracts::KnowledgeError::InvalidInput(_)
                | crate::ai::knowledge_contracts::KnowledgeError::NotFound
                | crate::ai::knowledge_contracts::KnowledgeError::RevisionConflict { .. },
            ) => "invalid_input",
            Self::CapabilityUnavailable(_) => "capability_unavailable",
            Self::InvalidInput(_) | Self::Validation { .. } => "invalid_input",
            Self::CredentialMissing => "credential_missing",
            Self::CredentialInvalid => "credential_invalid",
            Self::CredentialPermissions => "credential_permissions",
            Self::CredentialFileType => "credential_file_type",
            Self::CredentialIo(_) | Self::CredentialLocation(_) => "credential_io",
            Self::TemporaryUnavailable => "temporary_unavailable",
            Self::Timeout => "timeout",
            Self::Canceled => "canceled",
            Self::Provider(code) => code,
            Self::ToolOutcomeDiagnostic(_) => "tool_outcome_unknown",
            Self::CodexVersion { code, .. } => code,
            Self::Model { code, .. } => code,
        }
    }

    pub(crate) fn user_message(&self) -> String {
        match self {
            Self::NativeWorkflow { failure: error, .. } => {
                let mut parts = vec![error.code.clone()];
                for (name, value) in [
                    ("package", &error.package_key),
                    ("instance", &error.instance_id),
                    ("request", &error.request_id),
                    ("operation", &error.operation_id),
                ] {
                    if let Some(value) = value {
                        parts.push(format!("{name}={value}"));
                    }
                }
                format!("{}（{}）", error.message, parts.join("，"))
            }
            Self::RunCreationUnknown {
                run_id,
                phase,
                safe_cause,
                prior_code,
                sqlite_code,
                sqlite_extended_code,
            } => {
                let mut parts = vec![format!("阶段：{phase}"), format!("原因：{safe_cause}")];
                if let Some(code) = prior_code {
                    parts.push(format!("原错误码：{code}"));
                }
                if let Some(code) = sqlite_code {
                    parts.push(format!("SQLite：{code}"));
                }
                if let Some(code) = sqlite_extended_code {
                    parts.push(format!("SQLite 扩展码：{code}"));
                }
                format!(
                    "AI 运行创建结果尚未确认，请重启应用后核对原运行 {run_id}（{}）",
                    parts.join("；")
                )
            }
            Self::ToolOutcomeDiagnostic(detail) => format!(
                "{}（{detail}）",
                Self::Provider("tool_outcome_unknown").user_message()
            ),
            Self::CodexVersion { cause, .. } => cause.clone(),
            Self::Model { code, cause } => {
                format!("{}（{cause}）", Self::Provider(code).user_message())
            }
            Self::Storage(_) | Self::Database(_) => {
                "AI 运行数据保存失败，请重试；若持续失败请检查本地存储状态".to_owned()
            }
            Self::Knowledge(crate::ai::knowledge_contracts::KnowledgeError::Storage(_))
            | Self::Knowledge(crate::ai::knowledge_contracts::KnowledgeError::Database(_))
            | Self::Knowledge(crate::ai::knowledge_contracts::KnowledgeError::SemanticIndex(_)) => {
                "知识晋升失败，请重试；若持续失败请检查本地存储状态".to_owned()
            }
            Self::CredentialIo(_) | Self::CredentialLocation(_) => {
                "本地凭据文件无法安全读写；原文件未被修改".to_owned()
            }
            Self::Provider("authentication") => {
                "模型服务认证失败；请核对当前服务的登录状态、凭据和访问权限".to_owned()
            }
            Self::Provider("insufficient_balance") => {
                "provider 账户余额不足；请检查账户后重试".to_owned()
            }
            Self::Provider("rate_limited") => "provider 请求频率受限；请稍后重试".to_owned(),
            Self::Provider("invalid_request") => {
                "provider 拒绝了请求参数；请检查当前模型 ID、endpoint 和任务设置".to_owned()
            }
            Self::Provider("invalid_response") => {
                "provider 返回了无法解析的响应；本次结果未被采用".to_owned()
            }
            Self::Provider("tool_outcome_unknown") => {
                "工具执行结果尚未确认；请核对原请求，避免重复执行".to_owned()
            }
            Self::Provider("temporary_unavailable") => "provider 暂时不可用；请稍后重试".to_owned(),
            Self::Provider("forbidden_tool_call") => {
                "provider 返回了当前有界任务禁止的工具调用；本次结果未被采用".to_owned()
            }
            Self::Provider("truncated_output") => {
                "provider 输出达到长度限制，未采用不完整结果".to_owned()
            }
            Self::Provider("content_filtered") => {
                "provider 未返回可用内容；本次结果未被采用".to_owned()
            }
            Self::Provider(_) => "provider 请求失败；请检查设置或稍后重试".to_owned(),
            _ => self.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCommandError {
    code: String,
    message: String,
    #[serde(flatten)]
    details: CommandErrorDetails,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum CommandErrorDetails {
    Native {
        #[serde(rename = "nativeFailure", skip_serializing_if = "Option::is_none")]
        native_failure: Option<Box<crate::runtime_plugins::native::NativeHostError>>,
    },
    AnalysisWorkflow(serde_json::Map<String, serde_json::Value>),
}

impl AiCommandError {
    /// Accept only the existing Core workflow error envelope, preserving its
    /// phase, settlement, operation and Native diagnostic fields verbatim.
    pub(crate) fn from_analysis_workflow(value: serde_json::Value) -> Self {
        let serde_json::Value::Object(mut details) = value else {
            return Self::incompatible_workflow_error();
        };
        let Some(serde_json::Value::String(code)) = details.remove("code") else {
            return Self::incompatible_workflow_error();
        };
        let Some(serde_json::Value::String(message)) = details.remove("message") else {
            return Self::incompatible_workflow_error();
        };
        Self {
            code,
            message,
            details: CommandErrorDetails::AnalysisWorkflow(details),
        }
    }

    fn incompatible_workflow_error() -> Self {
        AiError::NativeWorkflow {
            code: "native_workflow_failed",
            failure: crate::runtime_plugins::native::NativeHostError::for_profile(
                crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
                "plugin_host_response_mismatch",
                "Core 论文分析错误缺少规范代码或说明。",
            ),
        }
        .into()
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl From<AiError> for AiCommandError {
    fn from(error: AiError) -> Self {
        let message = error.user_message();
        if let AiError::NativeWorkflow {
            failure: native_failure,
            ..
        } = error
        {
            return Self {
                code: native_failure.code.clone(),
                message,
                details: CommandErrorDetails::Native {
                    native_failure: Some(native_failure),
                },
            };
        }
        Self {
            code: error.code().into(),
            message,
            details: CommandErrorDetails::Native {
                native_failure: None,
            },
        }
    }
}

impl From<StorageError> for AiCommandError {
    fn from(error: StorageError) -> Self {
        AiError::Storage(error).into()
    }
}

impl From<research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError> for AiError {
    fn from(
        error: research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError,
    ) -> Self {
        use research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError;
        match error {
            AnalysisWorkflowError::InvalidInput(message) => Self::InvalidInput(message),
            AnalysisWorkflowError::TemporaryUnavailable => Self::TemporaryUnavailable,
        }
    }
}

// Shared pure failures retain their original variants in every Core composition.
impl From<research_workbench_native_sdk::candidate_contracts::CandidateValidationError>
    for AiError
{
    fn from(
        error: research_workbench_native_sdk::candidate_contracts::CandidateValidationError,
    ) -> Self {
        match error {
            research_workbench_native_sdk::candidate_contracts::CandidateValidationError::InvalidInput(message)=>Self::InvalidInput(message),
            research_workbench_native_sdk::candidate_contracts::CandidateValidationError::Validation{path,message}=>Self::Validation{path,message},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_failures_have_actionable_sanitized_messages() {
        for (code, expected_fragment) in [
            ("invalid_request", "模型 ID"),
            ("invalid_response", "无法解析"),
            ("temporary_unavailable", "稍后重试"),
            ("forbidden_tool_call", "禁止的工具调用"),
        ] {
            let message = AiError::Provider(code).user_message();
            assert!(message.contains(expected_fragment));
            assert!(!message.contains("http"));
        }
    }

    #[test]
    fn analysis_workflow_command_keeps_the_original_bounded_error_envelope() {
        let wire = serde_json::json!({
            "code":"data_operation_outcome_unknown", "message":"Original safe cause",
            "workflowId":"workflow", "analysisWorkflowOperation":"startAttempt",
            "analysisWorkflowPhase":"commit", "analysisWorkflowOutcome":"unknown",
            "nativeFailure":{"kind":"sourceDatabase","phase":"commit","outcome":"unknown",
                "packageKey":"selected-package","nativeRequestId":"request",
                "databaseError":{"code":"constraint","phase":"commit"}},
        });
        let command = AiCommandError::from_analysis_workflow(wire.clone());
        assert_eq!(command.code(), "data_operation_outcome_unknown");
        assert_eq!(command.message(), "Original safe cause");
        assert_eq!(serde_json::to_value(command).unwrap(), wire);
        let invalid =
            AiCommandError::from_analysis_workflow(serde_json::json!({"message":"no code"}));
        assert_eq!(invalid.code(), "plugin_host_response_mismatch");
    }

    #[test]
    fn original_command_wire_keeps_two_fields_and_native_fault_keeps_actual_identity() {
        use crate::runtime_plugins::native::{NativeHostError, NativeProfile};
        assert_eq!(
            serde_json::to_value(AiCommandError::from(AiError::InvalidInput(
                "original".into()
            )))
            .unwrap(),
            serde_json::json!({"code":"invalid_input", "message":"original"})
        );
        let mut failure = NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_source_changed",
            "Source changed",
        )
        .package("synthetic-package");
        failure.instance_id = Some("synthetic-instance".into());
        failure.request_id = Some("synthetic-request".into());
        let error = AiCommandError::from(AiError::NativeWorkflow {
            code: "native_workflow_failed",
            failure,
        });
        assert_eq!(error.code(), "plugin_host_source_changed");
        let wire = serde_json::to_value(error).unwrap();
        assert_eq!(wire["nativeFailure"]["packageKey"], "synthetic-package");
        assert_eq!(wire["nativeFailure"]["instanceId"], "synthetic-instance");
        assert_eq!(wire["nativeFailure"]["requestId"], "synthetic-request");
        assert!(
            wire["message"]
                .as_str()
                .unwrap()
                .contains("request=synthetic-request")
        );
    }
}
