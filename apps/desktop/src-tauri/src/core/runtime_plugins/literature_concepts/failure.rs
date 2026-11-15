use crate::{
    data_interface::{DataError, plugin_resources::PluginDataError},
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
};
use research_workbench_native_sdk::{
    concepts::ConceptRejectionV1, nal_bootstrap::BootstrapPhaseV1,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ConceptOutcome {
    NotStarted,
    RolledBack,
    Committed,
    Unknown,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum ConceptProblem {
    Domain(ConceptRejectionV1),
    Missing,
    RevisionConflict,
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Owner(&'static str),
    Binding,
    Codec,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConceptFailure {
    pub(crate) phase: &'static str,
    pub(crate) outcome: ConceptOutcome,
    pub(crate) concept_id: Option<String>,
    pub(crate) current_revision: Option<i64>,
    pub(crate) problem: Box<ConceptProblem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) semantic_phase: Option<BootstrapPhaseV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}
impl ConceptFailure {
    pub(crate) fn new(phase: &'static str, problem: ConceptProblem) -> Self {
        Self {
            phase,
            problem: Box::new(problem),
            outcome: ConceptOutcome::NotStarted,
            concept_id: None,
            current_revision: None,
            semantic_phase: None,
            rollback_error: None,
        }
    }
    pub(super) fn data(phase: &'static str, cause: Box<PluginDataError>) -> Self {
        Self::new(phase, ConceptProblem::Data(cause))
    }
    pub(super) fn sql(phase: &'static str, cause: &rusqlite::Error) -> Self {
        Self::new(
            phase,
            ConceptProblem::Database(Box::new(SafeDatabaseError::sql(phase, cause))),
        )
    }
    pub(crate) fn owner(cause: DataError) -> Self {
        match cause {
            DataError::Sql(e) => Self::sql("owner", &e),
            DataError::Unavailable => {
                Self::new("owner", ConceptProblem::Owner("DATA_RUNTIME_UNAVAILABLE"))
            }
            DataError::Pair(_) => {
                Self::new("owner", ConceptProblem::Owner("DATA_RUNTIME_PAIR_INVALID"))
            }
            DataError::Log(_) => Self::new("owner", ConceptProblem::Owner("DATA_RUNTIME_LOG")),
            DataError::Admission(_) => {
                Self::new("owner", ConceptProblem::Owner("DATA_RUNTIME_ADMISSION"))
            }
        }
    }
    pub(crate) fn unknown() -> Self {
        let mut f = Self::new(
            "execution",
            ConceptProblem::Owner("CONCEPT_WORKER_UNSETTLED"),
        );
        f.outcome = ConceptOutcome::Unknown;
        f
    }
    pub(crate) fn native(self) -> Box<NativeHostError> {
        let (code, mut message) = match self.problem.as_ref() {
            ConceptProblem::Domain(reason) => ("invalid_input", reason.message().to_owned()),
            ConceptProblem::Missing => ("not_found", "找不到研究对象或来源锚点".into()),
            ConceptProblem::RevisionConflict => (
                "revision_conflict",
                self.current_revision.map_or_else(
                    || "对象版本冲突".into(),
                    |current| format!("对象版本冲突；当前版本为 {current}"),
                ),
            ),
            ConceptProblem::Binding => (
                "plugin_host_response_mismatch",
                "概念准备与本次输入不匹配。".into(),
            ),
            _ => (
                "storage_error",
                "知识数据保存失败，请重试；若持续失败请检查本地存储状态".into(),
            ),
        };
        if self.outcome == ConceptOutcome::Committed {
            message = format!("概念已提交，返回读取失败；请刷新核对，禁止自动重放。{message}");
        }
        if self.outcome == ConceptOutcome::Unknown {
            message = format!("概念保存结果未知，禁止自动重试。{message}");
        }
        let mut e = NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message);
        e.current_revision = self.current_revision;
        if let ConceptProblem::Data(cause) = self.problem.as_ref() {
            e.data_error = Some(cause.clone());
        }
        e.concepts = Some(Box::new(self));
        e
    }
}
