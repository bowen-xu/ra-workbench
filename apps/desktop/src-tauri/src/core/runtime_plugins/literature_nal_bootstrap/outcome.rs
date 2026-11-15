use super::error::CaptureError;
use crate::runtime_plugins::native::SafeDatabaseError;
use research_workbench_native_sdk::nal_bootstrap::{BootstrapPhaseV1, BootstrapRejectionV1};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FailureContext {
    Network,
    Import,
    Repair,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum BootstrapCause {
    Capture(CaptureError),
    Database(Box<SafeDatabaseError>),
    Owner(&'static str),
    Rejected(BootstrapRejectionV1),
    InvalidStructure,
    MissingConcept,
    InvalidBinding(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BootstrapProblem {
    pub(crate) phase: BootstrapPhaseV1,
    pub(crate) context: FailureContext,
    pub(crate) cause: BootstrapCause,
}

impl BootstrapProblem {
    pub(super) fn capture(
        phase: BootstrapPhaseV1,
        context: FailureContext,
        cause: CaptureError,
    ) -> Self {
        Self {
            phase,
            context,
            cause: BootstrapCause::Capture(cause),
        }
    }

    pub(super) fn binding(phase: BootstrapPhaseV1, reason: &'static str) -> Self {
        Self {
            phase,
            context: FailureContext::Network,
            cause: BootstrapCause::InvalidBinding(reason),
        }
    }

    pub(super) fn structure(phase: BootstrapPhaseV1) -> Self {
        Self {
            phase,
            context: FailureContext::Network,
            cause: BootstrapCause::InvalidStructure,
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match &self.cause {
            BootstrapCause::InvalidBinding(_) => "plugin_host_response_mismatch",
            BootstrapCause::Capture(CaptureError::SnapshotChanged { .. }) => {
                "plugin_data_snapshot_changed"
            }
            _ if self.context != FailureContext::Network => "invalid_input",
            BootstrapCause::Rejected(_)
            | BootstrapCause::InvalidStructure
            | BootstrapCause::MissingConcept => "invalid_input",
            _ => "storage_error",
        }
    }
}

impl fmt::Display for BootstrapProblem {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.cause, BootstrapCause::InvalidBinding(_)) {
            return write!(out, "关系网络准备结果与本次调用不匹配。");
        }
        if matches!(
            self.cause,
            BootstrapCause::Capture(CaptureError::SnapshotChanged { .. })
        ) {
            return write!(out, "关系网络准备期间发生变化，请重新读取。");
        }
        match self.context {
            FailureContext::Import => write!(out, "旧归类概念迁移失败；原数据保持不变"),
            FailureContext::Repair => write!(out, "旧领域关系修正失败；原数据保持不变"),
            FailureContext::Network => match self.cause {
                BootstrapCause::InvalidStructure | BootstrapCause::Rejected(_) => {
                    write!(out, "关系数据结构无效")
                }
                BootstrapCause::MissingConcept => write!(out, "概念已不存在，请刷新"),
                _ => write!(out, "关系存储失败，请重试"),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum BootstrapOutcome {
    NotStarted,
    RolledBack,
    Committed {
        revision: i64,
    },
    Unknown {
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_revision: Option<i64>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BootstrapFailure {
    pub(crate) problem: BootstrapProblem,
    pub(crate) outcome: BootstrapOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl fmt::Display for BootstrapFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.outcome, BootstrapOutcome::Unknown { .. }) {
            write!(out, "关系网络提交或回滚结果未知，禁止自动重试。")
        } else {
            self.problem.fmt(out)
        }
    }
}

impl std::error::Error for BootstrapFailure {}
