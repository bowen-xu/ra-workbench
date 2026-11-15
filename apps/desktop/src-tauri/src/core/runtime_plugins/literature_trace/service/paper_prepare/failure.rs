//! Safe preparation errors retain original codes and prior screenshot progress.
use super::*;
use crate::{
    ai::{
        error::AiCommandError,
        paper_reading_contracts::PaperContextError,
        structured_resource_validation::{Failure, FailureCause},
    },
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::{literature_context::ContextFailure, literature_hbs::HbsOperationError},
};
use serde_json::{Value, json};
#[derive(Clone, Copy)]
pub(super) struct Progress {
    pub image_write_started: bool,
    pub saved_image_asset_count: usize,
}
#[derive(Debug)]
pub(super) enum Error {
    Boundary(ContextFailure),
    Native(Box<NativeHostError>),
    Conflict(Option<DecisionOrigin>),
}
impl Error {
    pub fn boundary(mut error: ContextFailure) -> Self {
        if let FailureCause::SourceData(cause) = &error.failure.cause
            && cause.original_sql_failure.is_some()
        {
            error.failure.error = AiError::PluginDatabase(cause.clone());
        }
        Self::Boundary(error)
    }
    pub fn original(error: AiError, phase: &'static str) -> Self {
        let failure = match error {
            AiError::Database(error) => {
                let cause = crate::runtime_plugins::native::SafeDatabaseError::sql(phase, &error);
                Failure {
                    error: AiError::Database(error),
                    cause: FailureCause::SourceDatabase(Box::new(cause)),
                }
            }
            error => Failure::original(error),
        };
        Self::Boundary(Box::new(BoundaryFailure {
            failure,
            phase,
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        }))
    }
    pub fn conflict(origin: Option<DecisionOrigin>) -> Self {
        Self::Conflict(origin)
    }
    pub fn rejected(message: String, origin: DecisionOrigin) -> Self {
        Self::Boundary(Box::new(BoundaryFailure {
            failure: Failure {
                error: AiError::InvalidInput(message),
                cause: FailureCause::NativeRejection(Box::new(origin)),
            },
            phase: "validation",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        }))
    }
    pub fn unsettled() -> Self {
        Self::Native(LiteratureTraceService::error(
            "plugin_host_paper_preparation_unsettled",
            "阅读资料准备未确认结算，请核对原快照与对象后重试。",
        ))
    }
    pub fn mismatch() -> Self {
        Self::Native(LiteratureTraceService::error(
            "plugin_host_response_mismatch",
            "阅读资料准备结果与本次阶段不符。",
        ))
    }
    pub fn needs_recovery(&self) -> bool {
        matches!(self, Self::Boundary(error) if error.needs_recovery() || error.rollback_error.is_some())
    }
    pub fn into_ai(self, package: &str, progress: Option<Progress>) -> AiError {
        use crate::runtime_plugins::literature_context::preparation::PreparationDiagnostic;
        let (code, message, diagnostic, mut native) = match self {
            Self::Native(error) => ("native_workflow_failed", error.message.clone(), None, error),
            Self::Conflict(origin) => {
                let error = AiError::InvalidInput(
                    "文献资料在准备期间发生变化，或当前解析尚未就绪，请核对后重试。".into(),
                );
                let mut native = LiteratureTraceService::error(error.code(), error.to_string());
                if let Some(origin) = origin {
                    native.instance_id = Some(origin.instance_id);
                    native.request_id = Some(origin.native_request_id);
                }
                (error.code(), error.to_string(), None, native)
            }
            Self::Boundary(boundary) => {
                let unknown = boundary.needs_recovery() || boundary.rollback_error.is_some();
                let code = if unknown {
                    "data_operation_outcome_unknown"
                } else {
                    boundary.failure.error.code()
                };
                let message = AiCommandError::from(boundary.failure.error)
                    .message()
                    .to_owned();
                let (message, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                    failure: Failure {
                        error: message,
                        cause: boundary.failure.cause,
                    },
                    phase: boundary.phase,
                    outcome: boundary.outcome,
                    rollback_error: boundary.rollback_error,
                })
                .into_wire();
                let native = LiteratureTraceService::error(code, message.clone());
                (code, message, diagnostic, native)
            }
        };
        native.package_key = Some(package.into());
        native.message = message;
        if let Some(value) = &diagnostic {
            native.instance_id = value.instance_id.clone();
            native.request_id = value.native_request_id.clone();
        }
        native.paper_preparation = Some(Box::new(PreparationDiagnostic {
            failure: diagnostic,
            image_object_write_started: progress.is_some_and(|value| value.image_write_started),
            saved_image_asset_count: progress.map_or(0, |value| value.saved_image_asset_count),
        }));
        AiError::NativeWorkflow {
            code,
            failure: native,
        }
    }
    pub fn into_wire(self, snapshot: bool, package: &str, progress: Option<Progress>) -> Value {
        let mut value = match self {
            Self::Native(mut error) => {
                error.package_key = Some(package.into());
                json!(error)
            }
            Self::Conflict(origin) => {
                let code = if snapshot {
                    json!(PaperContextError::Conflict)
                } else {
                    json!("invalid_input")
                };
                let error = AiError::InvalidInput(
                    "文献资料在准备期间发生变化，或当前解析尚未就绪，请核对后重试。".into(),
                );
                let cause = origin.map_or(FailureCause::Original, |origin| {
                    FailureCause::NativeRejection(Box::new(origin))
                });
                let (message, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                    failure: Failure {
                        error: error.to_string(),
                        cause,
                    },
                    phase: "validation",
                    outcome: HbsOutcome::NotStarted,
                    rollback_error: None,
                })
                .into_wire();
                let mut value = json!({"code":code,"message":message});
                if let Some(diagnostic) = diagnostic {
                    value["nativeFailure"] = json!(diagnostic);
                }
                value
            }
            Self::Boundary(error) => {
                let context_code = match &error.failure.error {
                    AiError::InvalidInput(_) => PaperContextError::InvalidInput,
                    _ => PaperContextError::Unavailable,
                };
                let Failure {
                    error: original_error,
                    cause,
                } = error.failure;
                let public_error = AiCommandError::from(original_error);
                let original = if snapshot {
                    json!({"code":context_code,"message":public_error.message()})
                } else {
                    json!(&public_error)
                };
                let (message, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                    failure: Failure {
                        error: public_error.message().to_owned(),
                        cause,
                    },
                    phase: error.phase,
                    outcome: error.outcome,
                    rollback_error: error.rollback_error,
                })
                .into_wire();
                match diagnostic {
                    None => original,
                    Some(native_failure) => {
                        json!({"code":original["code"],"message":message,"nativeFailure":native_failure})
                    }
                }
            }
        };
        if let Some(progress) = progress.filter(|progress| progress.image_write_started) {
            if !value.is_object() {
                value = json!({"code":value});
            }
            value["imageObjectWriteStarted"] = json!(progress.image_write_started);
            value["savedImageAssetCount"] = json!(progress.saved_image_asset_count);
        }
        if value.is_object() {
            value["paperContextOperation"] = json!("prepare");
        }
        value
    }
}
