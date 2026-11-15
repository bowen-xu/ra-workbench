use crate::{
    data_interface::{DataError, plugin_resources::PluginDataError},
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
};
use research_workbench_native_sdk::reference_records::ReferenceRecordRejectionV1;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ReferenceOutcome {
    NotStarted,
    RolledBack,
    Committed,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReferenceProgress {
    pub(crate) phase: &'static str,
    pub(crate) outcome: ReferenceOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_revision: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum ReferenceProblem {
    Domain(ReferenceRecordRejectionV1),
    Missing,
    RevisionConflict,
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Binding,
    Owner(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReferenceFailure {
    #[serde(flatten)]
    pub(crate) progress: ReferenceProgress,
    pub(crate) problem: ReferenceProblem,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl ReferenceProgress {
    pub(crate) fn new(phase: &'static str) -> Self {
        Self {
            phase,
            outcome: ReferenceOutcome::NotStarted,
            record_id: None,
            current_revision: None,
        }
    }

    pub(crate) fn at(&self, phase: &'static str) -> Self {
        Self {
            phase,
            ..self.clone()
        }
    }

    pub(crate) fn failure(&self, problem: ReferenceProblem) -> ReferenceFailure {
        ReferenceFailure {
            progress: self.clone(),
            problem,
            rollback_error: None,
        }
    }

    pub(crate) fn data(&self, cause: Box<PluginDataError>) -> ReferenceFailure {
        self.failure(ReferenceProblem::Data(cause))
    }

    pub(crate) fn database(&self, cause: &rusqlite::Error) -> ReferenceFailure {
        self.failure(ReferenceProblem::Database(Box::new(
            SafeDatabaseError::sql(self.phase, cause),
        )))
    }
}

impl ReferenceFailure {
    pub(crate) fn native(self) -> Box<NativeHostError> {
        let (code, message) = match &self.problem {
            ReferenceProblem::Domain(reason) => ("invalid_input", reason.message().into()),
            ReferenceProblem::Missing => ("not_found", "找不到参考条目".into()),
            ReferenceProblem::RevisionConflict => (
                "revision_conflict",
                match self.progress.current_revision {
                    Some(revision) => format!("参考条目版本冲突；当前版本为 v{revision}"),
                    None => "参考条目版本冲突".into(),
                },
            ),
            ReferenceProblem::Binding => (
                "plugin_host_response_mismatch",
                "参考记录准备与本次请求不匹配。".into(),
            ),
            ReferenceProblem::Data(cause) => (
                "storage_error",
                format!("研究综合数据保存失败：{}", cause.safe_cause),
            ),
            ReferenceProblem::Database(cause) => {
                ("storage_error", format!("研究综合数据保存失败：{cause}"))
            }
            ReferenceProblem::Owner(cause) => (
                "storage_error",
                format!("研究综合数据拥有者不可用：{cause}"),
            ),
        };
        let message = match self.progress.outcome {
            ReferenceOutcome::Committed => {
                format!("参考条目已提交，但结果读取失败；请刷新核对，禁止自动重放。{message}")
            }
            ReferenceOutcome::Unknown => {
                format!("参考条目操作结果未知，禁止自动重试。{message}")
            }
            _ => message,
        };
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message);
        error.current_revision = self.progress.current_revision;
        if let ReferenceProblem::Data(cause) = &self.problem {
            error.data_error = Some(cause.clone());
        }
        error.reference_records = Some(Box::new(self));
        error
    }

    pub(crate) fn owner(cause: DataError) -> Self {
        let problem = match cause {
            DataError::Sql(error) => {
                ReferenceProblem::Database(Box::new(SafeDatabaseError::sql("owner", &error)))
            }
            DataError::Unavailable => ReferenceProblem::Owner("DATA_RUNTIME_UNAVAILABLE"),
            DataError::Pair(_) => ReferenceProblem::Owner("DATA_RUNTIME_PAIR_INVALID"),
            DataError::Log(_) => ReferenceProblem::Owner("DATA_RUNTIME_LOG"),
            DataError::Admission(_) => ReferenceProblem::Owner("DATA_RUNTIME_ADMISSION"),
        };
        ReferenceProgress::new("owner").failure(problem)
    }

    pub(crate) fn unknown(phase: &'static str, cause: &'static str) -> Self {
        let mut progress = ReferenceProgress::new(phase);
        progress.outcome = ReferenceOutcome::Unknown;
        progress.failure(ReferenceProblem::Owner(cause))
    }
}
