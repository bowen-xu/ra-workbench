use crate::{
    hbs_storage::boundary::BoundaryFailure,
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
    storage::StorageError,
};
use research_workbench_native_sdk::evidence_write::{
    EvidenceWriteInputV1, EvidenceWriteRejectionV1,
};
use serde::Serialize;
use thiserror::Error;
#[derive(Debug, Error)]
pub(crate) enum ClaimsError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("证据数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("证据序列化失败：{0}")]
    Serialization(#[from] serde_json::Error),
    #[error("观点内容不能为空")]
    EmptyStatement,
    #[error("不支持的认识角色：{0}")]
    InvalidEpistemicRole(String),
    #[error("不支持的证据关系：{0}")]
    InvalidStance(String),
    #[error("找不到观点或原文锚点")]
    NotFound,
    #[error("观点版本冲突；当前版本为 {current_revision}")]
    RevisionConflict { current_revision: i64 },
    #[error("evidence validation pending")]
    Need(Box<EvidenceWriteInputV1>),
    #[error("{0}")]
    Technical(Box<NativeHostError>),
    #[error("evidence transaction settlement")]
    Settled(Box<BoundaryFailure<ClaimsError>>),
    #[error("evidence committed readback failed")]
    Readback {
        id: Option<String>,
        cause: Box<ClaimsError>,
    },
    #[error("evidence operation failed")]
    Identified { id: String, cause: Box<ClaimsError> },
    #[error("evidence statement failed")]
    StatementFailure(Box<ClaimsError>),
}

use crate::runtime_plugins::native::StorageMutationOutcome as EvidenceOutcome;
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EvidenceWriteDiagnostic {
    pub(crate) phase: &'static str,
    pub(crate) outcome: EvidenceOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) database_error: Option<Box<SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}
impl From<EvidenceWriteRejectionV1> for ClaimsError {
    fn from(reason: EvidenceWriteRejectionV1) -> Self {
        match reason {
            EvidenceWriteRejectionV1::EmptyStatement => Self::EmptyStatement,
            EvidenceWriteRejectionV1::InvalidEpistemicRole(value) => {
                Self::InvalidEpistemicRole(value)
            }
            EvidenceWriteRejectionV1::InvalidStance(value) => Self::InvalidStance(value),
        }
    }
}
impl From<super::super::literature_evidence::EvidenceReadError> for ClaimsError {
    fn from(cause: super::super::literature_evidence::EvidenceReadError) -> Self {
        match cause {
            super::super::literature_evidence::EvidenceReadError::Database(cause) => {
                Self::Database(cause)
            }
            super::super::literature_evidence::EvidenceReadError::NotFound => Self::NotFound,
        }
    }
}
impl ClaimsError {
    pub(crate) fn needs_recovery(&self) -> bool {
        match self {
            Self::Settled(failure) => failure.needs_recovery(),
            Self::Identified { cause, .. } => cause.needs_recovery(),
            _ => false,
        }
    }
    pub(crate) fn native(self) -> Box<NativeHostError> {
        let (code, message) = match &self {
            Self::EmptyStatement | Self::InvalidEpistemicRole(_) | Self::InvalidStance(_) => {
                ("invalid_input", self.to_string())
            }
            Self::NotFound => ("not_found", self.to_string()),
            Self::RevisionConflict { .. } => ("revision_conflict", self.to_string()),
            Self::Database(cause) => (
                "storage_error",
                format!(
                    "证据数据保存失败：{}",
                    SafeDatabaseError::sql("execution", cause)
                ),
            ),
            Self::Serialization(_) => ("storage_error", "证据数据序列化失败".into()),
            _ => ("storage_error", "证据操作未完成。".into()),
        };
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message);
        let mut diagnostic = EvidenceWriteDiagnostic {
            phase: "validation",
            outcome: EvidenceOutcome::NotStarted,
            object_id: None,
            database_error: None,
            rollback_error: None,
        };
        match self {
            Self::Technical(cause) => return cause,
            Self::Identified { id, cause } => {
                let mut error = cause.native();
                if let Some(d) = error.evidence_write.as_mut() {
                    d.object_id = Some(id);
                }
                return error;
            }
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
                diagnostic.outcome = failure.outcome.into();
                diagnostic.rollback_error = failure.rollback_error;
                error = failure.failure.error.native();
                diagnostic.database_error = error
                    .evidence_write
                    .as_ref()
                    .and_then(|d| d.database_error.clone());
                if matches!(diagnostic.outcome, EvidenceOutcome::Unknown) {
                    error.message = format!(
                        "证据保存结果未知，禁止自动重试，请重开后核对。{}",
                        error.message
                    );
                }
            }
            Self::Readback { id, cause } => {
                error = cause.native();
                diagnostic.phase = "postcommit-read";
                diagnostic.outcome = EvidenceOutcome::Committed;
                diagnostic.object_id = id;
                diagnostic.database_error = error
                    .evidence_write
                    .as_ref()
                    .and_then(|d| d.database_error.clone());
                error.message = format!(
                    "证据操作已提交但读回失败，请刷新核对，禁止自动重放。{}",
                    error.message
                );
            }
            Self::StatementFailure(cause) => {
                error = cause.native();
                diagnostic.phase = "link-insert";
                diagnostic.outcome = EvidenceOutcome::StatementFailed;
                diagnostic.database_error = error
                    .evidence_write
                    .as_ref()
                    .and_then(|d| d.database_error.clone());
            }
            _ => {}
        }
        error.evidence_write = Some(Box::new(diagnostic));
        error
    }
}
