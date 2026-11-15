use crate::{
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
};
use serde::Serialize;

/// An autocommit error is not a rollback claim: SQLite FAIL can retain writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum LifecycleOutcome {
    NotDispatched,
    StatementReturned { affected_rows: u64 },
    StatementFailed,
    RolledBack,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum LifecycleProblem {
    Missing,
    Binding,
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Owner(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LifecycleFailure {
    pub(crate) phase: &'static str,
    pub(crate) problem: LifecycleProblem,
    pub(crate) outcome: LifecycleOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl LifecycleFailure {
    pub(crate) fn new(
        phase: &'static str,
        problem: LifecycleProblem,
        outcome: LifecycleOutcome,
    ) -> Self {
        Self {
            phase,
            problem,
            outcome,
            rollback_error: None,
        }
    }

    pub(crate) fn native(self, batch: bool) -> Box<NativeHostError> {
        let (code, message) = match &self.problem {
            LifecycleProblem::Missing if batch => (
                "not_found",
                "部分文献已归档、删除或不存在，请刷新后重试".into(),
            ),
            LifecycleProblem::Missing => ("not_found", "找不到文献或附件".into()),
            LifecycleProblem::Binding => (
                "plugin_host_response_mismatch",
                "文献生命周期准备与本次请求不匹配。".into(),
            ),
            LifecycleProblem::Data(cause) => (
                "storage_error",
                format!("文献数据库操作失败：{}", cause.safe_cause),
            ),
            LifecycleProblem::Database(cause) => {
                ("storage_error", format!("文献数据库操作失败：{cause}"))
            }
            LifecycleProblem::Owner(_) => ("storage_error", "文献数据库操作未完成。".into()),
        };
        let message = if self.outcome == LifecycleOutcome::Unknown {
            format!("文献操作结果未知，禁止自动重试。{message}")
        } else if batch && code != "not_found" {
            format!("批量移入回收站失败，本次操作未提交，请重试：{message}")
        } else {
            message
        };
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message);
        if let LifecycleProblem::Data(cause) = &self.problem {
            error.data_error = Some(cause.clone());
        }
        error.literature_lifecycle = Some(Box::new(self));
        error
    }
}
