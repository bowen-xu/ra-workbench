//! Original index failures retain finite Native and transaction diagnostics.
use super::{decisions::Need, model::SemanticIndexError};
use crate::{
    ai::structured_resource_validation::{DecisionOrigin, Failure, Flow},
    hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome},
    runtime_plugins::native::NativeHostError,
};
use rusqlite::{Connection, Transaction};

#[derive(Debug)]
pub(crate) enum Error {
    Original(SemanticIndexError),
    Native {
        error: Box<NativeHostError>,
        outcome: HbsOutcome,
    },
    Rejected {
        message: String,
        origin: DecisionOrigin,
    },
    Conflict(DecisionOrigin),
    Need(Box<Need>),
    Boundary(Box<BoundaryFailure<Error>>),
    Unsettled,
}
impl From<SemanticIndexError> for Error {
    fn from(error: SemanticIndexError) -> Self {
        Self::Original(error)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        SemanticIndexError::from(error).into()
    }
}
impl From<crate::storage::StorageError> for Error {
    fn from(error: crate::storage::StorageError) -> Self {
        SemanticIndexError::from(error).into()
    }
}
impl From<Box<NativeHostError>> for Error {
    fn from(error: Box<NativeHostError>) -> Self {
        Self::Native {
            error,
            outcome: HbsOutcome::NotStarted,
        }
    }
}
impl Error {
    pub fn mismatch() -> Self {
        SemanticIndexError::EngineProtocol("index decision kind mismatch".into()).into()
    }
    pub fn needs_recovery(&self) -> bool {
        matches!(self, Self::Unsettled)
            || matches!(self, Self::Boundary(failure) if failure.needs_recovery())
    }

    pub(crate) fn into_wire(self, operation: &'static str, id: Option<&str>) -> serde_json::Value {
        use crate::runtime_plugins::{
            literature_hbs::HbsOperationError, native::SafeDatabaseError,
        };
        use serde_json::json;
        let mut wire = match self {
            Self::Original(error) => {
                let cause = match &error {
                    SemanticIndexError::Database(error) => {
                        Some(json!(SafeDatabaseError::sql("execution", error)))
                    }
                    SemanticIndexError::InvalidInput(message) => {
                        Some(json!({"code":"invalid_input","safeCause":message}))
                    }
                    SemanticIndexError::Provider(code) => Some(json!({"code":code})),
                    SemanticIndexError::EngineOperation { code }
                        if code.len() <= 100
                            && code
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
                    {
                        Some(json!({"code":code}))
                    }
                    _ => None,
                };
                let mut wire = original(error);
                if let Some(cause) = cause {
                    wire["indexCause"] = cause;
                }
                wire
            }
            Self::Boundary(failure) => {
                let failure = *failure;
                let wire = failure.failure.error.into_wire(operation, id);
                let message = wire["message"]
                    .as_str()
                    .expect("every index error wire variant has a message")
                    .to_owned();
                let (_, diagnostic) = HbsOperationError::Boundary(BoundaryFailure {
                    failure: Failure {
                        error: message,
                        cause: failure.failure.cause,
                    },
                    phase: failure.phase,
                    outcome: failure.outcome,
                    rollback_error: failure.rollback_error,
                })
                .into_wire();
                let mut wire = wire;
                wire["nativeFailure"] = json!(diagnostic);
                wire
            }
            Self::Native { error, outcome } => {
                let code = error.code.clone();
                let (message, diagnostic) =
                    HbsOperationError::native(error, "validation", outcome).into_wire();
                json!({"code":code,"message":message,"nativeFailure":diagnostic})
            }
            Self::Rejected { message, origin } => {
                json!({"code":"invalid_input","message":message,"packageKey":origin.package_key,"instanceId":origin.instance_id,"nativeRequestId":origin.native_request_id})
            }
            Self::Conflict(origin) => {
                json!({"code":"plugin_host_source_state_changed","message":"语义索引状态在等待规则期间已变化，请重新读取后重试。","packageKey":origin.package_key,"instanceId":origin.instance_id,"nativeRequestId":origin.native_request_id})
            }
            Self::Unsettled | Self::Need(_) => {
                json!({"code":"worker_unavailable","message":"语义索引工作未确认结算，请核对原任务状态。","outcome":"unknown"})
            }
        };
        wire["indexOperation"] = json!(operation);
        if let Some(id) = id {
            let field = match operation {
                "job" => "jobId",
                "retry" => "representationId",
                "rebuild" => "embeddingSpaceId",
                _ => "operationId",
            };
            wire[field] = json!(id);
        }
        wire
    }
}

pub(crate) fn original(error: SemanticIndexError) -> serde_json::Value {
    // The original command error consists solely of two serializable strings.
    serde_json::to_value(super::model::SemanticIndexCommandError::from(error))
        .expect("index command error has only string fields")
}

/// Preserve the original Deferred transaction. A missing decision can escape
/// only after rollback and autocommit are explicitly confirmed.
pub(super) fn deferred<T>(
    db: &mut Connection,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T, Error>,
) -> Result<T, Error> {
    let tx = db.transaction().map_err(|error| {
        let mut failure = Failure::sql(Error::Unsettled, "begin", &error);
        failure.error = Error::from(error);
        Error::Boundary(Box::new(BoundaryFailure {
            failure,
            phase: "begin",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        }))
    })?;
    let result = operation(&tx).map_err(|error| match error {
        Error::Need(need) => Flow::Need(need),
        Error::Original(SemanticIndexError::Database(error)) => {
            let mut failure = Failure::sql(Error::Unsettled, "write", &error);
            failure.error = Error::from(error);
            Flow::Fail(failure)
        }
        Error::Rejected { message, origin } => Flow::Fail(Failure {
            error: Error::Rejected {
                message,
                origin: origin.clone(),
            },
            cause: crate::ai::structured_resource_validation::FailureCause::NativeRejection(
                Box::new(origin),
            ),
        }),
        error => Flow::Fail(Failure::original(error)),
    });
    let settled = boundary::settle_with(tx, result, "write", true, true, Error::from, |_| {
        Error::Unsettled
    });
    boundary::confirm_with(db, settled, true, |_| Error::Unsettled).map_err(|error| match error {
        BoundaryError::Failure(failure) => Error::Boundary(Box::new(failure)),
        BoundaryError::Pause(pause) => Error::Need(pause.need()),
    })
}
