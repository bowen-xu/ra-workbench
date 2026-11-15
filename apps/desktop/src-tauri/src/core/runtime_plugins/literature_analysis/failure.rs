use crate::{
    ai::{
        error::{AiCommandError, AiError},
        structured_resource_validation::{DecisionOrigin, Failure, FailureCause},
    },
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::{
        literature_hbs::HbsOperationError,
        native::{NativeHostError, NativeProfile, SafeDatabaseError},
    },
};
use research_workbench_native_sdk::analysis_history::HistoryRequestV1;
use serde_json::{Value, json};

pub(crate) fn original(error: AiError) -> Value {
    json!(AiCommandError::from(error))
}

pub(crate) enum Error {
    Original(AiError),
    Native(Box<NativeHostError>),
    Rejected {
        message: String,
        origin: DecisionOrigin,
    },
}
impl From<AiError> for Error {
    fn from(error: AiError) -> Self {
        Self::Original(error)
    }
}
impl From<crate::storage::StorageError> for Error {
    fn from(error: crate::storage::StorageError) -> Self {
        Self::Original(error.into())
    }
}
impl From<Box<NativeHostError>> for Error {
    fn from(error: Box<NativeHostError>) -> Self {
        Self::Native(error)
    }
}
impl Error {
    pub(crate) fn technical(code: &str, message: &str) -> Self {
        Self::Native(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            code,
            message,
        ))
    }
    pub(crate) fn unsettled() -> Self {
        Self::technical(
            "plugin_host_source_worker_unsettled",
            "论文分析历史读取未确认完成。",
        )
    }
    pub(crate) fn mismatch() -> Self {
        Self::technical(
            "plugin_host_response_mismatch",
            "论文分析历史结果与本次输入不匹配。",
        )
    }
    pub(crate) fn into_ai(self, package: &str) -> AiError {
        match self {
            Self::Original(error) => error,
            Self::Native(mut failure) => {
                if failure.package_key.is_none() {
                    failure.package_key = Some(package.into());
                }
                AiError::NativeWorkflow {
                    code: "native_workflow_failed",
                    failure,
                }
            }
            Self::Rejected { message, origin } => {
                let mut failure = NativeHostError::for_profile(
                    NativeProfile::NoteTraceDecision,
                    "invalid_input",
                    message,
                )
                .package(&origin.package_key);
                failure.instance_id = Some(origin.instance_id);
                failure.request_id = Some(origin.native_request_id);
                AiError::NativeWorkflow {
                    code: "invalid_input",
                    failure,
                }
            }
        }
    }
    pub(crate) fn into_wire(self, request: &HistoryRequestV1, package: Option<&str>) -> Value {
        let mut wire = match self {
            Self::Rejected { message, origin } => {
                let (_, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                    failure: Failure {
                        error: message.clone(),
                        cause: FailureCause::NativeRejection(Box::new(origin)),
                    },
                    phase: "source-read",
                    outcome: HbsOutcome::NotStarted,
                    rollback_error: None,
                })
                .into_wire();
                json!({"code":"invalid_input", "message":message, "nativeFailure":diagnostic})
            }
            Self::Original(error) => {
                let cause = match &error {
                    AiError::Database(sql) => {
                        Some(Box::new(SafeDatabaseError::sql("source-read", sql)))
                    }
                    _ => None,
                };
                let command = AiCommandError::from(error);
                let mut wire = json!(&command);
                if let Some(cause) = cause {
                    let (_, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                        failure: Failure {
                            error: command.message().to_owned(),
                            cause: FailureCause::SourceDatabase(cause),
                        },
                        phase: "source-read",
                        outcome: HbsOutcome::NotStarted,
                        rollback_error: None,
                    })
                    .into_wire();
                    wire["nativeFailure"] = json!(diagnostic);
                }
                wire
            }
            Self::Native(mut error) => {
                if error.package_key.is_none() {
                    error.package_key = package.map(str::to_owned);
                }
                let code = error.code.clone();
                let message = error.message.clone();
                let (_, native_failure) =
                    HbsOperationError::native(error, "source-read", HbsOutcome::NotStarted)
                        .into_wire();
                json!({"code":code, "message":message, "nativeFailure":native_failure})
            }
        };
        match request {
            HistoryRequestV1::Get { workflow_id } => {
                wire["analysisHistoryOperation"] = json!("get");
                wire["workflowId"] = json!(workflow_id);
            }
            HistoryRequestV1::Latest { paper_id } => {
                wire["analysisHistoryOperation"] = json!("latest");
                wire["paperId"] = json!(paper_id);
            }
        }
        wire
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_boundary_keeps_existing_native_identity_and_original_null_shape() {
        let failure = || {
            let mut failure = NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                "plugin_host_source_changed",
                "Safe source changed",
            )
            .package("selected-package");
            failure.instance_id = Some("actual-instance".into());
            failure.request_id = Some("actual-request".into());
            AiError::NativeWorkflow {
                code: "native_workflow_failed",
                failure,
            }
        };
        let wire = crate::runtime_plugins::literature_analysis::workflow_failure::original(
            failure(),
            "source-read",
            HbsOutcome::NotStarted,
        );
        assert_eq!(wire["code"], "plugin_host_source_changed");
        assert_eq!(wire["nativeFailure"]["packageKey"], "selected-package");
        assert_eq!(wire["nativeFailure"]["instanceId"], "actual-instance");
        assert_eq!(wire["nativeFailure"]["requestId"], "actual-request");
        let boundary = crate::runtime_plugins::literature_analysis::workflow_failure::boundary(
            BoundaryFailure {
                failure: Failure::original(super::super::error::AnalysisError::Core(
                    failure().into(),
                )),
                phase: "write",
                outcome: HbsOutcome::RolledBack,
                rollback_error: None,
            },
        );
        assert_eq!(boundary["nativeFailure"], wire["nativeFailure"]);
        let original = crate::runtime_plugins::literature_analysis::workflow_failure::original(
            AiError::InvalidInput("original cause".into()),
            "validation",
            HbsOutcome::NotStarted,
        );
        assert!(original.as_object().unwrap().contains_key("nativeFailure"));
        assert!(original["nativeFailure"].is_null());
        assert_eq!(original["message"], "original cause");
    }

    #[test]
    fn history_read_retains_the_actual_native_message_and_read_only_outcome() {
        let message = "所选文献包未声明原论文分析历史投影能力。";
        let request = HistoryRequestV1::Latest {
            paper_id: "paper".into(),
        };
        let wire = Error::technical("plugin_host_capability_unavailable", message)
            .into_wire(&request, Some(&"a".repeat(64)));
        assert_eq!(wire["message"], message);
        assert_eq!(wire["nativeFailure"]["outcome"], "notStarted");
        assert_eq!(wire["nativeFailure"]["phase"], "source-read");
        assert_eq!(wire["paperId"], "paper");
    }
}
