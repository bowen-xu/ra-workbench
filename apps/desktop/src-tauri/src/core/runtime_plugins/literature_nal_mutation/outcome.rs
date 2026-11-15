use crate::{
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::{
        literature_nal_annotation::AnnotationProblem, literature_nal_bootstrap::BootstrapProblem,
        native::SafeDatabaseError,
    },
};
use research_workbench_native_sdk::nal_mutation::NalMutationRejection;
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MutationPhase {
    Begin,
    Startup,
    RevisionCheck,
    Mutation,
    TermRead,
    KindUsage,
    RelationKind,
    StatementInsert,
    Link,
    Revision,
    FinalLoad,
    Commit,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum MutationCause {
    Startup(Box<BootstrapProblem>),
    Link(Box<AnnotationProblem>),
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Rejected(NalMutationRejection),
    Conflict,
    Binding(&'static str),
    Owner(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationProblem {
    pub(crate) phase: MutationPhase,
    pub(crate) cause: MutationCause,
}

impl MutationProblem {
    pub(crate) fn binding(phase: MutationPhase, reason: &'static str) -> Self {
        Self {
            phase,
            cause: MutationCause::Binding(reason),
        }
    }

    pub(crate) fn rejected(phase: MutationPhase, reason: NalMutationRejection) -> Self {
        Self {
            phase,
            cause: MutationCause::Rejected(reason),
        }
    }

    pub(crate) fn sql(
        phase: MutationPhase,
        operation: &'static str,
        error: &rusqlite::Error,
    ) -> Self {
        Self {
            phase,
            cause: MutationCause::Database(Box::new(SafeDatabaseError::sql(operation, error))),
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match &self.cause {
            MutationCause::Startup(problem) => problem.code(),
            MutationCause::Link(problem) => problem.code(),
            MutationCause::Rejected(_) => "invalid_input",
            MutationCause::Conflict => "version_conflict",
            MutationCause::Binding(_) => "plugin_host_response_mismatch",
            _ => "storage_error",
        }
    }
}

impl fmt::Display for MutationProblem {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            MutationCause::Startup(problem) => problem.fmt(out),
            MutationCause::Link(problem) => problem.fmt(out),
            // Presentation of the closed Native rejection; policy remains in
            // its canonical pure implementation, not this error adapter.
            MutationCause::Rejected(reason) => out.write_str(match reason {
                NalMutationRejection::TermStructureTooLarge => "词项结构过大或嵌套过深",
                NalMutationRejection::ProductArity => "乘积需要 2–16 个有序词项",
                NalMutationRejection::InvalidTermIdentity => "词项身份无效",
                NalMutationRejection::MissingTerm => "概念或活跃文献已不存在，请刷新",
                NalMutationRejection::InvalidConceptKind => "概念类型无效",
                NalMutationRejection::ConceptUsedAsRelation => {
                    "该概念已用作关系；请先移除相关关系语句和规则"
                }
                NalMutationRejection::NotRelation => "只有关系概念可以定义传递规则",
                NalMutationRejection::InvalidStatementCount => "每次添加 1–500 条语句",
                NalMutationRejection::InvalidStatementIdentityOrOrigin => "新语句身份或来源无效",
                NalMutationRejection::MissingConcept => "概念已不存在，请刷新",
                NalMutationRejection::MissingStatement => "语句已不存在",
            }),
            MutationCause::Conflict => out.write_str("关系已变化，请刷新后重试"),
            MutationCause::Binding(_) => out.write_str("关系修改准备结果与本次调用不匹配。"),
            _ => out.write_str("关系存储失败，请重试"),
        }
    }
}

/// The revision comes only from an actual final load. It is never a durable
/// receipt or proof that an uncertain commit rolled back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum MutationOutcome {
    NotStarted,
    RolledBack,
    Committed { revision: i64 },
    Unknown { observed_revision: Option<i64> },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationFailure {
    pub(crate) problem: MutationProblem,
    pub(crate) outcome: MutationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl fmt::Display for MutationFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.outcome, MutationOutcome::Unknown { .. }) {
            out.write_str("关系修改提交或回滚结果未知，禁止自动重试。")
        } else {
            self.problem.fmt(out)
        }
    }
}

impl std::error::Error for MutationFailure {}
