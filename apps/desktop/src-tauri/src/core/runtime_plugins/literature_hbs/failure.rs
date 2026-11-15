use crate::{
    ai::structured_resource_validation::FailureCause,
    data_interface::plugin_resources::PluginDataError,
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::native::{NativeHostError, NativeTerminationDiagnostic, SafeDatabaseError},
};
use serde::Serialize;

pub(crate) enum HbsOperationError {
    Boundary(BoundaryFailure),
    Native {
        error: Box<NativeHostError>,
        phase: &'static str,
        outcome: HbsOutcome,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeFailure {
    pub kind: &'static str,
    pub phase: &'static str,
    pub outcome: HbsOutcome,
    pub plugin_id: &'static str,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_error: Option<Box<PluginDataError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_error: Option<Box<SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_error: Option<Box<SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_error: Option<NativeTerminationDiagnostic>,
}
impl NativeFailure {
    fn empty(kind: &'static str, phase: &'static str, outcome: HbsOutcome, code: String) -> Self {
        Self {
            kind,
            phase,
            outcome,
            plugin_id: "literature_library",
            code,
            package_key: None,
            instance_id: None,
            native_request_id: None,
            operation_id: None,
            exit_code: None,
            signal: None,
            data_error: None,
            database_error: None,
            rollback_error: None,
            termination_error: None,
        }
    }
}
impl HbsOperationError {
    pub(crate) fn native(
        error: Box<NativeHostError>,
        phase: &'static str,
        outcome: HbsOutcome,
    ) -> Self {
        Self::Native {
            error,
            phase,
            outcome,
        }
    }
    pub(crate) fn unknown() -> Self {
        Self::native(
            NativeHostError::for_profile(
                crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
                "structured_hbs_worker_unsettled",
                "结构化 HBS 操作未确认结算，禁止自动重放。",
            ),
            "execution",
            HbsOutcome::Unknown,
        )
    }
    /// Keep the original MODEL text independent from the technical cause.
    pub(crate) fn into_wire(self) -> (String, Option<NativeFailure>) {
        match self {
            Self::Native {
                error,
                phase,
                outcome,
            } => {
                let cause = super::native_message::safe_message(&error.message).map(str::to_owned);
                let mut diagnostic = NativeFailure::empty("technical", phase, outcome, error.code);
                diagnostic.package_key = error.package_key;
                diagnostic.instance_id = error.instance_id;
                diagnostic.native_request_id = error.request_id;
                diagnostic.operation_id = error.operation_id;
                diagnostic.exit_code = error.exit_code;
                diagnostic.signal = error.signal;
                diagnostic.data_error = error.data_error;
                diagnostic.termination_error = error.termination_error;
                let message = match cause {
                    Some(cause) => format!("MODEL_STORAGE_ERROR：{cause}"),
                    None => format!(
                        "MODEL_STORAGE_ERROR：{}；原内容与保存请求已保留。",
                        diagnostic.code
                    ),
                };
                (message, Some(diagnostic))
            }
            Self::Boundary(boundary) => {
                let mut diagnostic = match boundary.failure.cause {
                    FailureCause::Original
                        if boundary.rollback_error.is_none()
                            && !matches!(
                                boundary.outcome,
                                HbsOutcome::Unknown | HbsOutcome::Committed
                            ) =>
                    {
                        return (boundary.failure.error, None);
                    }
                    FailureCause::Original => NativeFailure::empty(
                        "technical",
                        boundary.phase,
                        boundary.outcome,
                        local_postcommit_cause(&boundary.failure.error)
                            .map(|(code, _)| code)
                            .unwrap_or("structured_hbs_settlement_unknown")
                            .into(),
                    ),
                    FailureCause::NativeRejection(origin) => {
                        let mut failure = NativeFailure::empty(
                            "pureRejected",
                            boundary.phase,
                            boundary.outcome,
                            "invalid_input".into(),
                        );
                        failure.package_key = Some(origin.package_key);
                        failure.instance_id = Some(origin.instance_id);
                        failure.native_request_id = Some(origin.native_request_id);
                        failure
                    }
                    FailureCause::SourceData(error) => {
                        let mut failure = NativeFailure::empty(
                            "sourceData",
                            boundary.phase,
                            boundary.outcome,
                            error.code.into(),
                        );
                        failure.package_key = Some(error.package_key.clone());
                        failure.data_error = Some(error);
                        failure
                    }
                    FailureCause::SourceDatabase(error) => {
                        let mut failure = NativeFailure::empty(
                            "sourceData",
                            boundary.phase,
                            boundary.outcome,
                            error.code.into(),
                        );
                        failure.database_error = Some(error);
                        failure
                    }
                    FailureCause::SourceReadUnavailable => NativeFailure::empty(
                        "sourceData",
                        boundary.phase,
                        boundary.outcome,
                        "structured_hbs_source_unavailable".into(),
                    ),
                };
                diagnostic.rollback_error = boundary.rollback_error;
                let message = if diagnostic.kind == "pureRejected" {
                    boundary.failure.error
                } else {
                    safe_boundary_message(&boundary.failure.error, diagnostic.kind)
                };
                (message, Some(diagnostic))
            }
        }
    }
}

fn safe_boundary_message(original: &str, kind: &str) -> String {
    // The original Flow error may contain rusqlite Display text. Preserve only
    // its fixed MODEL context; typed Data/Database fields carry the real cause.
    if let Some((code, cause)) = local_postcommit_cause(original) {
        return format!("MODEL_STORAGE_ERROR：{code}：{cause}");
    }
    let context = original.split('：').next().filter(|value| {
        matches!(
            *value,
            "MODEL_INVALID_DOCUMENT"
                | "MODEL_INVALID_BASELINE"
                | "MODEL_RECOVERY_INVALID"
                | "MODEL_STORAGE_ERROR"
                | "MODEL_NOT_FOUND"
                | "MODEL_VERSION_CONFLICT"
        )
    });
    let summary = if kind == "sourceData" {
        "结构化操作的数据访问失败。"
    } else {
        "结构化操作未能按原请求完成；请核对结算诊断。"
    };
    match context {
        Some(context) => format!("MODEL_STORAGE_ERROR：{context}；{summary}"),
        None => format!("MODEL_STORAGE_ERROR：{summary}"),
    }
}

fn local_postcommit_cause(original: &str) -> Option<(&'static str, &'static str)> {
    // These are the original get/decode owner's fixed messages, not driver text.
    match original {
        "MODEL_STORAGE_ERROR：not_found：找不到结构化资源" => {
            Some(("not_found", "找不到结构化资源"))
        }
        "MODEL_STORAGE_ERROR：storage_error：结构化资源存储不可用" => {
            Some(("storage_error", "结构化资源存储不可用"))
        }
        "MODEL_STORAGE_ERROR：invalid_input：结构化资源存储格式无效" => {
            Some(("invalid_input", "结构化资源存储格式无效"))
        }
        _ => None,
    }
}
