use crate::{
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::{literature_nal_bootstrap::BootstrapProblem, native::SafeDatabaseError},
};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MergePhase {
    Header,
    Preparation,
    Begin,
    Outer,
    Search,
    Titles,
    Timestamp,
    Nal,
    Classification,
    Snapshot,
    History,
    Commit,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum MergeCause {
    Missing,
    Binding(&'static str),
    SnapshotCodec(String),
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Nal(Box<BootstrapProblem>),
    Owner(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergeProblem {
    pub(crate) phase: MergePhase,
    pub(crate) cause: MergeCause,
}

impl MergeProblem {
    pub(crate) fn binding(phase: MergePhase, reason: &'static str) -> Self {
        Self {
            phase,
            cause: MergeCause::Binding(reason),
        }
    }
    pub(crate) fn sql(phase: MergePhase, operation: &'static str, error: &rusqlite::Error) -> Self {
        Self {
            phase,
            cause: MergeCause::Database(Box::new(SafeDatabaseError::sql(operation, error))),
        }
    }
    pub(crate) fn data(phase: MergePhase, cause: Box<PluginDataError>) -> Self {
        if matches!(phase, MergePhase::Nal) {
            return Self {
                phase,
                cause: MergeCause::Nal(Box::new(BootstrapProblem {
                    phase:
                        research_workbench_native_sdk::nal_bootstrap::BootstrapPhaseV1::FinalLoad,
                    context:
                        crate::runtime_plugins::literature_nal_bootstrap::FailureContext::Network,
                    cause:
                        crate::runtime_plugins::literature_nal_bootstrap::BootstrapCause::Capture(
                            crate::runtime_plugins::literature_nal_bootstrap::CaptureError::Data(
                                cause,
                            ),
                        ),
                })),
            };
        }
        Self {
            phase,
            cause: MergeCause::Data(cause),
        }
    }
    pub(crate) fn code(&self) -> &'static str {
        match self.cause {
            MergeCause::Missing => "not_found",
            MergeCause::Binding(_) => "plugin_host_response_mismatch",
            MergeCause::SnapshotCodec(_) => "connector_error",
            _ => "storage_error",
        }
    }
}
impl fmt::Display for MergeProblem {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            MergeCause::Missing => out.write_str("找不到文献或附件"),
            MergeCause::Binding(_) => out.write_str("合并准备与本次候选不匹配。"),
            MergeCause::SnapshotCodec(cause) => write!(out, "DOI 元数据请求失败：{cause}"),
            MergeCause::Data(cause) => write!(out, "文献数据库操作失败：{}", cause.safe_cause),
            MergeCause::Database(cause) => write!(out, "文献数据库操作失败：{cause}"),
            MergeCause::Nal(cause) => write!(out, "文献数据库操作失败：{cause}"),
            MergeCause::Owner(_) => out.write_str("文献数据库操作失败，请重试"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MergeOutcome {
    NotStarted,
    RolledBack,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergeFailure {
    pub(crate) problem: MergeProblem,
    pub(crate) outcome: MergeOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation_id: Option<String>,
}
impl fmt::Display for MergeFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.outcome == MergeOutcome::Unknown {
            out.write_str("合并提交或回滚结果未知，禁止自动重试。")
        } else {
            self.problem.fmt(out)
        }
    }
}
impl std::error::Error for MergeFailure {}
