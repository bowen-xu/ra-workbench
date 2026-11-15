//! One finite structured-operation owner; resource errors retain their typed code.
use super::{HbsOperation, HbsOperationError, NativeFailure};
use crate::{
    ai::structured_resource_validation::Failure,
    hbs_storage::boundary::{BoundaryError, BoundaryFailure, HbsNeed, HbsOutcome},
    structured_resources::{
        AdoptionDraft, Error, HistoryDraft, Resource, ReviewDraft, WriteDraft, WriteResult,
    },
};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

pub(crate) enum StructuredOperation {
    Hbs(Arc<HbsOperation>),
    Review(ReviewDraft),
    History(HistoryDraft),
    Write(WriteDraft),
    TextWrite(WriteDraft),
    BindTemplate(AdoptionDraft),
}
pub(crate) enum StructuredOutput {
    Hbs(Value),
    Resource(Box<Resource>),
    Write(Box<WriteResult>),
}
pub(crate) enum OperationFailure {
    Hbs(String),
    Resource(Error),
}
pub(crate) enum OperationError {
    Existing(HbsOperationError),
    Resource(BoundaryFailure<Error>),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceOperationError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_failure: Option<Box<NativeFailure>>,
}
impl OperationError {
    pub(crate) fn into_write(self, request: &str) -> ResourceOperationError {
        let mut error = self.into_resource(request);
        if error.native_failure.is_some() || error.code == "storage_error" {
            error
                .message
                .push_str("；请重试原保存请求，核对已提交草稿和正式发布结果。");
        }
        error
    }
    pub(crate) fn unknown() -> Self {
        Self::Existing(HbsOperationError::unknown())
    }
    pub(crate) fn into_hbs(self) -> HbsOperationError {
        match self {
            Self::Existing(error) => error,
            Self::Resource(_) => HbsOperationError::unknown(),
        }
    }
    pub(crate) fn into_resource(self, request: &str) -> ResourceOperationError {
        let (code, message, native_failure) = match self {
            Self::Existing(error) => {
                let (message, diagnostic) = error.into_wire();
                ("storage_error", message, diagnostic)
            }
            Self::Resource(error) => {
                let original_code = error.failure.error.command_code();
                let phase = error.phase;
                let mapped = map_failure(error, |value| resource_message(value, phase));
                let unsettled =
                    matches!(mapped.outcome, HbsOutcome::Unknown | HbsOutcome::Committed);
                let (message, diagnostic) = HbsOperationError::Boundary(mapped).into_wire();
                let technical = unsettled
                    || diagnostic
                        .as_ref()
                        .is_some_and(|value| value.kind != "pureRejected");
                (
                    if technical {
                        "storage_error"
                    } else {
                        original_code
                    },
                    message,
                    diagnostic,
                )
            }
        };
        ResourceOperationError {
            code,
            message,
            request_id: Some(request.to_owned()),
            native_failure: native_failure.map(Box::new),
        }
    }
}
impl From<HbsOperationError> for OperationError {
    fn from(error: HbsOperationError) -> Self {
        Self::Existing(error)
    }
}

pub(crate) fn operation_error(error: BoundaryFailure<OperationFailure>) -> OperationError {
    let BoundaryFailure {
        failure: Failure { error, cause },
        phase,
        outcome,
        rollback_error,
    } = error;
    match error {
        OperationFailure::Hbs(error) => {
            OperationError::Existing(HbsOperationError::Boundary(BoundaryFailure {
                failure: Failure { error, cause },
                phase,
                outcome,
                rollback_error,
            }))
        }
        OperationFailure::Resource(error) => OperationError::Resource(BoundaryFailure {
            failure: Failure { error, cause },
            phase,
            outcome,
            rollback_error,
        }),
    }
}
pub(crate) fn map_boundary<E>(
    error: BoundaryError<HbsNeed, E>,
    project: impl FnOnce(E) -> OperationFailure,
) -> BoundaryError<HbsNeed, OperationFailure> {
    match error {
        BoundaryError::Pause(pause) => BoundaryError::Pause(pause),
        BoundaryError::Failure(failure) => BoundaryError::Failure(map_failure(failure, project)),
    }
}
fn map_failure<E, F>(
    failure: BoundaryFailure<E>,
    project: impl FnOnce(E) -> F,
) -> BoundaryFailure<F> {
    BoundaryFailure {
        failure: failure.failure.map_error(project),
        phase: failure.phase,
        outcome: failure.outcome,
        rollback_error: failure.rollback_error,
    }
}

fn resource_message(error: Error, phase: &str) -> String {
    let message = error.to_string();
    // These fixed messages come from the original postcommit draft reader.
    // Preserve its real code/cause without accepting arbitrary driver text.
    if phase == "postcommit-read"
        && matches!(
            (error.command_code(), message.as_str()),
            ("not_found", "找不到结构化资源")
                | ("storage_error", "结构化资源存储不可用")
                | ("invalid_input", "结构化资源存储格式无效")
        )
    {
        format!("MODEL_STORAGE_ERROR：{}：{message}", error.command_code())
    } else {
        message
    }
}

#[cfg(test)]
mod tests;
