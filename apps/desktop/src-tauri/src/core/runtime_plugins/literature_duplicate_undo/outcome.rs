use crate::{
    data_interface::plugin_resources::PluginDataError, runtime_plugins::native::SafeDatabaseError,
};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UndoPhase {
    Header,
    Snapshot,
    Preparation,
    Begin,
    ClassificationLoad,
    ClassificationCheck,
    NalLoad,
    NalCheck,
    NalDelete,
    NalRevision,
    ClassificationDelete,
    ClassificationRevision,
    Restore,
    Search,
    Commit,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum UndoCause {
    Missing,
    SnapshotCodec(String),
    Conflict,
    Structure,
    StructureCodec(UndoJsonCodec),
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Binding(&'static str),
    BindingCodec {
        reason: &'static str,
        codec: UndoJsonCodec,
    },
    Owner(&'static str),
}

/// JSON diagnostics without values, unknown field names or document fragments.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UndoJsonCodec {
    pub(crate) category: &'static str,
    pub(crate) line: usize,
    pub(crate) column: usize,
}

impl UndoJsonCodec {
    fn from_error(error: &serde_json::Error) -> Self {
        let category = match error.classify() {
            serde_json::error::Category::Io => "io",
            serde_json::error::Category::Syntax => "syntax",
            serde_json::error::Category::Data => "data",
            serde_json::error::Category::Eof => "eof",
        };
        Self {
            category,
            line: error.line(),
            column: error.column(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UndoProblem {
    pub(crate) phase: UndoPhase,
    pub(crate) cause: UndoCause,
}

impl UndoProblem {
    pub(crate) fn binding(phase: UndoPhase, cause: &'static str) -> Self {
        Self {
            phase,
            cause: UndoCause::Binding(cause),
        }
    }
    pub(crate) fn binding_codec(
        phase: UndoPhase,
        reason: &'static str,
        error: &serde_json::Error,
    ) -> Self {
        Self {
            phase,
            cause: UndoCause::BindingCodec {
                reason,
                codec: UndoJsonCodec::from_error(error),
            },
        }
    }
    pub(crate) fn structure_codec(phase: UndoPhase, error: &serde_json::Error) -> Self {
        Self {
            phase,
            cause: UndoCause::StructureCodec(UndoJsonCodec::from_error(error)),
        }
    }
    pub(crate) fn sql(phase: UndoPhase, operation: &'static str, error: &rusqlite::Error) -> Self {
        Self {
            phase,
            cause: UndoCause::Database(Box::new(SafeDatabaseError::sql(operation, error))),
        }
    }
    pub(crate) fn data(phase: UndoPhase, cause: Box<PluginDataError>) -> Self {
        Self {
            phase,
            cause: UndoCause::Data(cause),
        }
    }
    pub(crate) fn code(&self) -> &'static str {
        match &self.cause {
            UndoCause::Missing => "not_found",
            UndoCause::SnapshotCodec(_) => "connector_error",
            UndoCause::Conflict => "invalid_input",
            UndoCause::Binding(_) | UndoCause::BindingCodec { .. } => {
                "plugin_host_response_mismatch"
            }
            _ => "storage_error",
        }
    }
}

impl fmt::Display for UndoProblem {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            UndoCause::Missing => out.write_str("找不到文献或附件"),
            UndoCause::SnapshotCodec(cause) => write!(out, "DOI 元数据请求失败：{cause}"),
            UndoCause::Conflict => {
                out.write_str("合并后的分类已修改，请先恢复分类到合并后的状态再撤销合并")
            }
            UndoCause::Binding(_) | UndoCause::BindingCodec { .. } => {
                out.write_str("合并撤销准备与本次持久快照不匹配。")
            }
            UndoCause::Data(cause) => write!(out, "文献数据库操作失败：{}", cause.safe_cause),
            UndoCause::Database(cause) => write!(out, "文献数据库操作失败：{cause}"),
            UndoCause::Structure | UndoCause::StructureCodec(_) => {
                out.write_str("文献数据库操作失败：NAL 关系结构解码失败")
            }
            _ => out.write_str("文献数据库操作失败，请重试"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UndoOutcome {
    NotStarted,
    RolledBack,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UndoFailure {
    pub(crate) problem: UndoProblem,
    pub(crate) outcome: UndoOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl fmt::Display for UndoFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.outcome == UndoOutcome::Unknown {
            out.write_str("合并撤销提交或回滚结果未知，禁止自动重试。")
        } else {
            self.problem.fmt(out)
        }
    }
}
impl std::error::Error for UndoFailure {}
