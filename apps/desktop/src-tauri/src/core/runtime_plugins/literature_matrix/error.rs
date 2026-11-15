use crate::{
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
};
use research_workbench_native_sdk::evidence_matrix::MatrixValidationInputV1;

#[derive(Debug, thiserror::Error)]
pub(crate) enum MatrixError {
    #[error("研究综合数据保存失败")]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    InvalidInput(String),
    #[error("找不到参考条目")]
    NotFound,
    #[error("参考条目版本冲突；当前版本为 v{current_revision}")]
    RevisionConflict { current_revision: i64 },
    #[error("matrix validation pending")]
    Need(Box<MatrixValidationInputV1>),
    #[error("{0}")]
    Technical(Box<NativeHostError>),
    #[error("matrix transaction settlement")]
    Settled(Box<BoundaryFailure<MatrixError>>),
    #[error("matrix committed readback failed")]
    Readback {
        id: String,
        revision: Option<i64>,
        cause: Box<MatrixError>,
    },
    #[error("matrix operation failed")]
    Identified { id: String, cause: Box<MatrixError> },
}
pub(crate) use crate::runtime_plugins::native::StorageMutationDiagnostic as MatrixDiagnostic;
impl MatrixError {
    pub(crate) fn needs_recovery(&self) -> bool {
        match self {
            Self::Settled(failure) => failure.needs_recovery(),
            Self::Identified { cause, .. } => cause.needs_recovery(),
            _ => false,
        }
    }
    pub(crate) fn native(self) -> Box<NativeHostError> {
        let (code, message) = match &self {
            Self::InvalidInput(message) => ("invalid_input", message.clone()),
            Self::NotFound => ("not_found", self.to_string()),
            Self::RevisionConflict { .. } => ("revision_conflict", self.to_string()),
            Self::Database(cause) => (
                "storage_error",
                format!(
                    "研究综合数据保存失败：{}",
                    SafeDatabaseError::sql("execution", cause)
                ),
            ),
            _ => (
                "plugin_host_response_mismatch",
                "证据矩阵操作未完成。".into(),
            ),
        };
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message);
        let mut diagnostic = MatrixDiagnostic {
            phase: "validation",
            outcome: HbsOutcome::NotStarted,
            object_id: None,
            revision: None,
            database_error: None,
            rollback_error: None,
        };
        match self {
            Self::Identified { id, cause } => {
                let mut error = cause.native();
                if let Some(diagnostic) = error.evidence_matrix.as_mut() {
                    diagnostic.object_id = Some(id);
                }
                return error;
            }
            Self::Technical(cause) => return cause,
            Self::RevisionConflict { current_revision } => {
                error.current_revision = Some(current_revision)
            }
            Self::Database(cause) => {
                diagnostic.phase = "execution";
                diagnostic.database_error =
                    Some(Box::new(SafeDatabaseError::sql("execution", &cause)));
            }
            Self::Settled(failure) => {
                diagnostic.phase = failure.phase;
                diagnostic.outcome = failure.outcome;
                diagnostic.rollback_error = failure.rollback_error;
                error = failure.failure.error.native();
                diagnostic.database_error = error
                    .evidence_matrix
                    .as_ref()
                    .and_then(|d| d.database_error.clone());
                if diagnostic.outcome == HbsOutcome::Unknown {
                    error.message = format!(
                        "矩阵操作结果未知，禁止自动重试；请重开后核对。{}",
                        error.message
                    );
                }
            }
            Self::Readback {
                id,
                revision,
                cause,
            } => {
                error = cause.native();
                diagnostic.phase = "postcommit-read";
                diagnostic.outcome = HbsOutcome::Committed;
                diagnostic.object_id = Some(id);
                diagnostic.revision = revision;
                diagnostic.database_error = error
                    .evidence_matrix
                    .as_ref()
                    .and_then(|d| d.database_error.clone());
                error.message = format!(
                    "矩阵操作已提交但读回失败，请刷新核对，禁止自动重放。{}",
                    error.message
                );
            }
            _ => {}
        }
        error.evidence_matrix = Some(Box::new(diagnostic));
        error
    }
}
