use crate::{
    data_interface::plugin_resources::PluginDataError,
    notes::NotesError,
    runtime_plugins::{literature_nal_bootstrap::BootstrapProblem, native::SafeDatabaseError},
};
use research_workbench_native_sdk::{
    nal_annotation::NalAnnotation,
    nal_annotation_prepare::{NalAnnotationAfterLoadRejection, NalAnnotationBeforeLoadRejection},
};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AnnotationPhase {
    Validation,
    Begin,
    Startup,
    Decision,
    CreateNote,
    LinkIdentity,
    LinkLoad,
    LinkConflict,
    LinkNoteActive,
    LinkTarget,
    LinkInsert,
    Revision,
    Commit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AnnotationRejection {
    InvalidRequest,
    TargetMissing,
    InvalidAnnotationId,
    IdentityConflict,
    TargetConflict,
    InactiveNote,
    InvalidTargetKind,
}

impl From<NalAnnotationBeforeLoadRejection> for AnnotationRejection {
    fn from(reason: NalAnnotationBeforeLoadRejection) -> Self {
        match reason {
            NalAnnotationBeforeLoadRejection::InvalidAnnotationId => Self::InvalidAnnotationId,
        }
    }
}

impl From<NalAnnotationAfterLoadRejection> for AnnotationRejection {
    fn from(reason: NalAnnotationAfterLoadRejection) -> Self {
        match reason {
            NalAnnotationAfterLoadRejection::IdentityConflict => Self::IdentityConflict,
            NalAnnotationAfterLoadRejection::TargetConflict => Self::TargetConflict,
        }
    }
}

impl fmt::Display for AnnotationRejection {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(match self {
            Self::InvalidRequest => "笔记目标或标题无效",
            Self::TargetMissing => "笔记目标已不存在",
            Self::InvalidAnnotationId => "笔记关联身份无效",
            Self::IdentityConflict => "笔记关联身份冲突",
            Self::TargetConflict => "每个概念或语句只能关联一份笔记",
            Self::InactiveNote => "笔记不存在或不在活跃状态",
            Self::InvalidTargetKind => "笔记目标类型无效",
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    content = "diagnostic",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum AnnotationCause {
    Startup(Box<BootstrapProblem>),
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Rejected(AnnotationRejection),
    NoteCreation {
        code: &'static str,
        safe_cause: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        database: Option<Box<SafeDatabaseError>>,
    },
    Binding(&'static str),
    AnnotationSnapshotChanged,
    Owner(&'static str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnnotationProblem {
    pub(crate) phase: AnnotationPhase,
    pub(crate) cause: AnnotationCause,
}

impl AnnotationProblem {
    pub(crate) fn binding(phase: AnnotationPhase, reason: &'static str) -> Self {
        Self {
            phase,
            cause: AnnotationCause::Binding(reason),
        }
    }

    pub(crate) fn rejected(phase: AnnotationPhase, reason: AnnotationRejection) -> Self {
        Self {
            phase,
            cause: AnnotationCause::Rejected(reason),
        }
    }

    pub(crate) fn sql(
        phase: AnnotationPhase,
        operation: &'static str,
        error: &rusqlite::Error,
    ) -> Self {
        Self {
            phase,
            cause: AnnotationCause::Database(Box::new(SafeDatabaseError::sql(operation, error))),
        }
    }

    pub(crate) fn note(error: NotesError) -> Self {
        // This fixed caller passes a root-page draft, with no paper or parent.
        // Preserve known safe creation causes without retaining SQLite text,
        // titles, parameters, or a future NotesError's arbitrary payload.
        let (code, safe_cause, database) = match error {
            NotesError::InvalidInput(reason) if reason == "笔记标题过长" => {
                ("invalid_input", "笔记标题过长", None)
            }
            NotesError::InvalidInput(_) => ("invalid_input", "core_note_invalid_input", None),
            NotesError::Database(error) => (
                "storage_error",
                "sqlite_operation_failed",
                Some(Box::new(SafeDatabaseError::sql("create_note", &error))),
            ),
            _ => ("note_create_failed", "core_note_writer_failed", None),
        };
        Self {
            phase: AnnotationPhase::CreateNote,
            cause: AnnotationCause::NoteCreation {
                code,
                safe_cause,
                database,
            },
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match &self.cause {
            AnnotationCause::Startup(problem) => problem.code(),
            AnnotationCause::Binding(_) => "plugin_host_response_mismatch",
            AnnotationCause::AnnotationSnapshotChanged => "plugin_data_snapshot_changed",
            AnnotationCause::Rejected(_) | AnnotationCause::NoteCreation { .. } => "invalid_input",
            _ => "storage_error",
        }
    }
}

impl fmt::Display for AnnotationProblem {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            AnnotationCause::Startup(problem) => problem.fmt(out),
            AnnotationCause::Rejected(reason) => reason.fmt(out),
            AnnotationCause::NoteCreation { .. } => out.write_str("笔记创建失败，请重试"),
            AnnotationCause::Binding(_) => out.write_str("笔记关联准备结果与本次调用不匹配。"),
            AnnotationCause::AnnotationSnapshotChanged => {
                out.write_str("笔记关联准备期间关系已变化，请重新读取。")
            }
            _ => out.write_str("关系存储失败，请重试"),
        }
    }
}

/// Ensure returns an Annotation, not a State or a revision receipt. A known
/// commit may retain that original result; unknown never invents a revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum AnnotationOutcome {
    NotStarted,
    RolledBack,
    Committed { annotation: Box<NalAnnotation> },
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnnotationFailure {
    pub(crate) problem: AnnotationProblem,
    pub(crate) outcome: AnnotationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl fmt::Display for AnnotationFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.outcome, AnnotationOutcome::Unknown) {
            out.write_str("笔记关联提交或回滚结果未知，禁止自动重试。")
        } else {
            self.problem.fmt(out)
        }
    }
}

impl std::error::Error for AnnotationFailure {}
