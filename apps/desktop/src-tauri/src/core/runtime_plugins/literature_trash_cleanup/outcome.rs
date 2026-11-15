use crate::{
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum TrashCleanupOutcome {
    NotStarted,
    RolledBack,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum TrashCleanupProblem {
    Binding,
    PlanCycle,
    NotFound,
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Owner(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrashCleanupFailure {
    pub(crate) phase: &'static str,
    pub(crate) problem: TrashCleanupProblem,
    pub(crate) outcome: TrashCleanupOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) item_id: Option<String>,
    pub(crate) committed_item_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl TrashCleanupFailure {
    pub(crate) fn new(
        phase: &'static str,
        problem: TrashCleanupProblem,
        outcome: TrashCleanupOutcome,
    ) -> Self {
        Self {
            phase,
            problem,
            outcome,
            item_id: None,
            committed_item_ids: Vec::new(),
            rollback_error: None,
        }
    }

    pub(crate) fn native(self) -> Box<NativeHostError> {
        let (code, message) = match &self.problem {
            TrashCleanupProblem::Binding => (
                "plugin_host_response_mismatch",
                "回收站清理准备与固定契约不匹配。".into(),
            ),
            TrashCleanupProblem::NotFound => ("not_found", "找不到文献或附件".into()),
            TrashCleanupProblem::PlanCycle => (
                "storage_error",
                "文献数据库操作失败：Invalid parameter name: LITERATURE_CLEANUP_PLAN_CYCLE".into(),
            ),
            TrashCleanupProblem::Data(cause) => (
                "storage_error",
                format!("文献数据库操作失败：{}", cause.safe_cause),
            ),
            TrashCleanupProblem::Database(cause) => {
                ("storage_error", format!("文献数据库操作失败：{cause}"))
            }
            TrashCleanupProblem::Owner(_) => ("storage_error", "文献数据库操作未完成。".into()),
        };
        let message = if self.outcome == TrashCleanupOutcome::Unknown {
            format!("回收站清理结果未知，禁止自动重试。{message}")
        } else {
            message
        };
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message);
        if let TrashCleanupProblem::Data(cause) = &self.problem {
            error.data_error = Some(cause.clone());
        }
        error.trash_cleanup = Some(Box::new(self));
        error
    }
}

impl std::fmt::Display for TrashCleanupFailure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "trash_cleanup_failed ({})", self.phase)
    }
}
impl std::error::Error for TrashCleanupFailure {}
