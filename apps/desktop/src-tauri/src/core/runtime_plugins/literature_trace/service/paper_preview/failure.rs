//! Original domain codes with the existing safe source and Native diagnostics.
use super::*;
use crate::{
    ai::structured_resource_validation::Failure, hbs_storage::boundary::BoundaryFailure,
    runtime_plugins::literature_hbs::HbsOperationError,
};
use serde::Serialize;
use serde_json::json;
#[derive(Debug)]
pub(super) enum Error {
    Boundary(Box<BoundaryFailure<PaperContextError>>),
    Sources(Box<BoundaryFailure<reading_sources::Error>>),
    Submission(Box<BoundaryFailure<crate::agent::reading_submission::Error>>),
    PaperHost {
        error: Box<NativeHostError>,
        outcome: HbsOutcome,
    },
    Native(Box<NativeHostError>),
}
impl Error {
    pub fn into_analysis_ai(self, package: &str) -> crate::ai::error::AiError {
        use crate::ai::{error::AiError, structured_resource_validation::FailureCause};
        let mut native = match self {
            Self::Native(mut error) => {
                if error.package_key.is_none() {
                    error.package_key = Some(package.into());
                }
                return AiError::NativeWorkflow {
                    code: "native_workflow_failed",
                    failure: error,
                };
            }
            Self::Boundary(boundary) => {
                let original = crate::ai::analysis_support::AnalysisSupportError::paper_context(
                    boundary.failure.error,
                )
                .into_ai_error();
                let mut error =
                    LiteratureTraceService::error(original.code(), original.to_string())
                        .package(package);
                match boundary.failure.cause {
                    FailureCause::Original => return original,
                    FailureCause::NativeRejection(origin) => {
                        error.instance_id = Some(origin.instance_id);
                        error.request_id = Some(origin.native_request_id);
                    }
                    FailureCause::SourceData(cause) => error.data_error = Some(cause),
                    FailureCause::SourceDatabase(cause) => error.query_database_error = Some(cause),
                    FailureCause::SourceReadUnavailable => {
                        error
                            .message
                            .push_str("；structured_hbs_source_unavailable");
                    }
                }
                return AiError::NativeWorkflow {
                    code: original.code(),
                    failure: error,
                };
            }
            _ => LiteratureTraceService::error(
                "plugin_host_response_mismatch",
                "分析资料读取结果种类不匹配。",
            ),
        };
        native.package_key = Some(package.into());
        AiError::NativeWorkflow {
            code: "native_workflow_failed",
            failure: native,
        }
    }
    pub fn domain(error: PaperContextError) -> Self {
        Self::Boundary(Box::new(BoundaryFailure {
            failure: Failure::original(error),
            phase: "read",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        }))
    }
    pub fn sources(error: reading_sources::Error) -> Self {
        Self::source_failure(Failure::original(error))
    }
    pub fn source_failure<E: Into<reading_sources::Error>>(failure: Failure<E>) -> Self {
        Self::Sources(Box::new(BoundaryFailure {
            failure: failure.map_error(Into::into),
            phase: "read",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        }))
    }
    pub fn sql_failure(
        failure: Failure<crate::data_interface::SqlImmediateFailure>,
        phase: &'static str,
    ) -> Self {
        let outcome = if matches!(
            &failure.error,
            crate::data_interface::SqlImmediateFailure::Unknown(_)
        ) {
            HbsOutcome::Unknown
        } else if phase == "write"
            && !matches!(
                failure.cause,
                crate::ai::structured_resource_validation::FailureCause::Original
            )
        {
            // Non-original source failures at this phase came from execute_sql
            // after its original transaction confirmed rollback.
            HbsOutcome::RolledBack
        } else {
            HbsOutcome::NotStarted
        };
        Self::Submission(Box::new(BoundaryFailure {
            failure: failure.map_error(crate::agent::reading_submission::storage_error),
            phase,
            outcome,
            rollback_error: None,
        }))
    }
    pub fn paper_unsettled() -> Self {
        Self::PaperHost {
            error: LiteratureTraceService::error(
                "plugin_host_paper_submission_worker_unsettled",
                "论文提交工作未确认结算，请核对原请求与回执后重试。",
            ),
            outcome: HbsOutcome::Unknown,
        }
    }
    pub fn needs_recovery(&self) -> bool {
        match self {
            Self::Boundary(error) => error.needs_recovery(),
            Self::Sources(error) => error.needs_recovery(),
            Self::Submission(error) => error.needs_recovery(),
            Self::PaperHost { outcome, .. } => *outcome == HbsOutcome::Unknown,
            Self::Native(_) => false,
        }
    }
    pub fn unsettled() -> Self {
        Self::Native(LiteratureTraceService::error(
            "plugin_host_paper_preview_worker_unsettled",
            "论文预览工作未确认结算，请核对原快照后重试。",
        ))
    }
    pub fn mismatch() -> Self {
        Self::Native(LiteratureTraceService::error(
            "plugin_host_response_mismatch",
            "论文快照读取结果与本次操作不符。",
        ))
    }
    pub fn into_wire_with_package(mut self, key: &str) -> Value {
        if let Self::Native(error) = &mut self {
            error.package_key.get_or_insert_with(|| key.into());
        }
        self.into_wire()
    }
    pub fn into_wire(self) -> Value {
        let mut value = match self {
            Self::Native(error) | Self::PaperHost { error, .. } => json!(error),
            Self::Submission(error) => json!(error.failure.error),
            Self::Boundary(error) => boundary_wire(*error),
            Self::Sources(error) => boundary_wire(*error),
        };
        if value.is_object() {
            value["paperContextOperation"] = json!("preview");
        }
        value
    }
    pub fn into_submission_error(
        self,
        key: &str,
        request: &crate::agent::reading_submission::PrepareRequest,
    ) -> crate::agent::reading_submission::Error {
        use crate::agent::reading_submission::{Error as PaperError, SourceFailure};
        match self {
            Self::Native(error) => Self::PaperHost {
                error,
                outcome: HbsOutcome::NotStarted,
            }
            .into_submission_error(key, request),
            Self::PaperHost { mut error, outcome } => {
                error.package_key.get_or_insert_with(|| key.into());
                let (original, code) = if outcome == HbsOutcome::Unknown {
                    (PaperError::worker_unsettled(request), None)
                } else {
                    (PaperError::Unavailable, Some(error.code.clone()))
                };
                let (message, diagnostic) =
                    HbsOperationError::native(error, "execution", outcome).into_wire();
                SourceFailure::attach(original, message, diagnostic, code, request)
            }
            Self::Submission(error) => {
                let BoundaryFailure {
                    failure,
                    phase,
                    outcome,
                    rollback_error,
                } = *error;
                let original = failure.error;
                let (message, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                    failure: Failure {
                        error: original.to_string(),
                        cause: failure.cause,
                    },
                    phase,
                    outcome,
                    rollback_error,
                })
                .into_wire();
                SourceFailure::attach(original, message, diagnostic, None, request)
            }
            Self::Boundary(error) => Self::Submission(Box::new(BoundaryFailure {
                failure: error
                    .failure
                    .map_error(crate::agent::reading_submission::preview_error),
                phase: error.phase,
                outcome: error.outcome,
                rollback_error: error.rollback_error,
            }))
            .into_submission_error(key, request),
            Self::Sources(error) => Self::Submission(Box::new(BoundaryFailure {
                failure: error.failure.map_error(|_| PaperError::Unavailable),
                phase: error.phase,
                outcome: error.outcome,
                rollback_error: error.rollback_error,
            }))
            .into_submission_error(key, request),
        }
    }
}
fn boundary_wire<E: Serialize + ToString>(error: BoundaryFailure<E>) -> Value {
    let code = json!(error.failure.error);
    let (message, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
        failure: error.failure.map_error(|error| error.to_string()),
        phase: error.phase,
        outcome: error.outcome,
        rollback_error: error.rollback_error,
    })
    .into_wire();
    match diagnostic {
        None => code,
        Some(native_failure) => {
            json!({"code": code, "message": message, "nativeFailure": native_failure})
        }
    }
}
