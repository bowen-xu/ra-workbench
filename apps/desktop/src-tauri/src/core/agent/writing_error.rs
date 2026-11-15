//! Pure writing error contract; no command registration or runtime code.
use crate::storage::StorageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum WritingError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("写作数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("写作状态序列化失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("ZIP 工程处理失败：{0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("稿件文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("研究项目不存在")]
    ProjectNotFound,
    #[error("稿件不存在")]
    ManuscriptNotFound,
    #[error("请先归档稿件，再永久删除")]
    ManuscriptMustBeArchived,
    #[error("稿件信息无效")]
    InvalidManuscript,
    #[error("稿件标识无效")]
    InvalidManuscriptId,
    #[error("稿件路径无效")]
    InvalidPath,
    #[error("稿件文件不存在")]
    FileNotFound,
    #[error("文件已在其他位置被修改，请重新载入后再保存")]
    FileConflict,
    #[error("写作文件检查点格式无效")]
    InvalidCheckpoint,
    #[error("写作文件检查点已变化，请重新核对当前版本")]
    CheckpointConflict,
    #[error("文本文件超过 8 MiB 限制")]
    FileTooLarge,
    #[error("第一版只支持 UTF-8 文本文件")]
    UnsupportedEncoding,
    #[error("稿件工程不允许符号链接")]
    SymlinkNotAllowed,
    #[error("本机没有可用的 latexmk 与 LaTeX engine")]
    ToolchainUnavailable,
    #[error("该稿件已有编译任务正在运行")]
    CompileAlreadyRunning,
    #[error("编译任务不存在")]
    CompileRunNotFound,
    #[error("LaTeX engine 只允许 pdflatex、xelatex 或 lualatex")]
    InvalidEngine,
    #[error("编译进程失败：{0}")]
    Process(String),
    #[error("引用来源不存在或已移入回收站")]
    CitationSourceNotFound,
    #[error("literature_citations_unavailable：当前未装配文献管理插件；原引用与稿内书目已保留")]
    LiteratureUnavailable,
    #[error("Citation.js 必须提供单个有效 BibTeX entry")]
    InvalidBibtexEntry,
    #[error("受管 references.bib 已被手工修改，请先选择恢复生成版本或转为用户文件")]
    BibliographyConflict,
    #[error("BibTeX 文件由参考文献管理器维护，不能直接编辑：{0}")]
    ManagedBibliography(String),
    #[error("BibTeX 导入未完成：{0}")]
    BibliographyImport(String),
    #[error("当前成功编译没有可用的 SyncTeX 数据")]
    SynctexUnavailable,
    #[error("SyncTeX 没有找到对应位置")]
    SynctexNoResult,
}

impl From<crate::scoped_text_files::TextFileError> for WritingError {
    fn from(error: crate::scoped_text_files::TextFileError) -> Self {
        use crate::scoped_text_files::TextFileError as E;
        match error {
            E::Io(error) => Self::Io(error),
            E::InvalidManuscriptId => Self::InvalidManuscriptId,
            E::InvalidPath => Self::InvalidPath,
            E::FileNotFound => Self::FileNotFound,
            E::FileConflict => Self::FileConflict,
            E::FileTooLarge => Self::FileTooLarge,
            E::UnsupportedEncoding => Self::UnsupportedEncoding,
            E::SymlinkNotAllowed => Self::SymlinkNotAllowed,
        }
    }
}

impl WritingError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            WritingError::ProjectNotFound
            | WritingError::ManuscriptNotFound
            | WritingError::FileNotFound => "not_found",
            WritingError::InvalidManuscript
            | WritingError::ManuscriptMustBeArchived
            | WritingError::InvalidManuscriptId
            | WritingError::InvalidPath => "invalid_input",
            WritingError::FileConflict => "file_conflict",
            WritingError::InvalidCheckpoint => "invalid_input",
            WritingError::CheckpointConflict => "checkpoint_conflict",
            WritingError::FileTooLarge => "file_too_large",
            WritingError::UnsupportedEncoding => "unsupported_encoding",
            WritingError::SymlinkNotAllowed => "unsafe_path",
            WritingError::ToolchainUnavailable => "toolchain_unavailable",
            WritingError::CompileAlreadyRunning => "compile_already_running",
            WritingError::CompileRunNotFound => "compile_run_not_found",
            WritingError::InvalidEngine => "invalid_engine",
            WritingError::CitationSourceNotFound => "citation_source_not_found",
            WritingError::LiteratureUnavailable => "literature_citations_unavailable",
            WritingError::InvalidBibtexEntry => "invalid_bibtex_entry",
            WritingError::BibliographyConflict => "bibliography_conflict",
            WritingError::ManagedBibliography(_) => "managed_bibliography",
            WritingError::BibliographyImport(_) => "bibliography_import",
            WritingError::SynctexUnavailable => "synctex_unavailable",
            WritingError::SynctexNoResult => "synctex_no_result",
            WritingError::Process(_) => "compile_process_error",
            WritingError::Storage(StorageError::QueryWorker(_)) => "db_query_worker_failed",
            WritingError::Storage(_)
            | WritingError::Database(_)
            | WritingError::Json(_)
            | WritingError::Zip(_)
            | WritingError::Io(_) => "storage_error",
        }
    }
}

impl From<research_workbench_native_sdk::writing_bibliography::mutation::Rejection>
    for WritingError
{
    fn from(
        reason: research_workbench_native_sdk::writing_bibliography::mutation::Rejection,
    ) -> Self {
        use research_workbench_native_sdk::writing_bibliography::{
            BibliographyError, mutation::Rejection as R,
        };
        match reason {
            R::InvalidManuscript => Self::InvalidManuscript,
            R::Bibliography(BibliographyError::InvalidPath) => Self::InvalidPath,
            R::Bibliography(BibliographyError::InvalidBibtexEntry) => Self::InvalidBibtexEntry,
            R::Bibliography(BibliographyError::BibliographyImport(cause)) => {
                Self::BibliographyImport(cause)
            }
            R::ManagedBibliography(path) => Self::ManagedBibliography(path),
            R::FileConflict => Self::FileConflict,
            R::CitationSourceNotFound => Self::CitationSourceNotFound,
            R::LiteratureUnavailable => Self::LiteratureUnavailable,
        }
    }
}

impl From<research_workbench_native_sdk::writing_compile::Rejection> for WritingError {
    fn from(reason: research_workbench_native_sdk::writing_compile::Rejection) -> Self {
        use research_workbench_native_sdk::writing_compile::Rejection;
        match reason {
            Rejection::InvalidPath => Self::InvalidPath,
            Rejection::SynctexNoResult => Self::SynctexNoResult,
            Rejection::InvalidEngine => Self::InvalidEngine,
            Rejection::ToolchainUnavailable => Self::ToolchainUnavailable,
        }
    }
}

impl From<research_workbench_native_sdk::writing_project::Rejection> for WritingError {
    fn from(error: research_workbench_native_sdk::writing_project::Rejection) -> Self {
        use research_workbench_native_sdk::writing_project::Rejection as ProjectError;
        match error {
            ProjectError::ProjectNotFound => Self::ProjectNotFound,
            ProjectError::ManuscriptNotFound => Self::ManuscriptNotFound,
            ProjectError::FileNotFound => Self::FileNotFound,
            ProjectError::ManuscriptMustBeArchived => Self::ManuscriptMustBeArchived,
            ProjectError::InvalidManuscript => Self::InvalidManuscript,
            ProjectError::InvalidManuscriptId => Self::InvalidManuscriptId,
            ProjectError::InvalidPath => Self::InvalidPath,
            ProjectError::InvalidCheckpoint => Self::InvalidCheckpoint,
            ProjectError::CheckpointConflict => Self::CheckpointConflict,
            ProjectError::FileTooLarge => Self::FileTooLarge,
            ProjectError::CompileAlreadyRunning => Self::CompileAlreadyRunning,
            ProjectError::ManagedBibliography(path) => Self::ManagedBibliography(path),
        }
    }
}

impl From<research_workbench_native_sdk::writing_bibliography::rekey::Rejection> for WritingError {
    fn from(reason: research_workbench_native_sdk::writing_bibliography::rekey::Rejection) -> Self {
        use research_workbench_native_sdk::writing_bibliography::{
            BibliographyError, rekey::Rejection as R,
        };
        match reason {
            R::Project(reason) => reason.into(),
            R::CitationSourceNotFound => Self::CitationSourceNotFound,
            R::Bibliography(BibliographyError::InvalidPath) => Self::InvalidPath,
            R::Bibliography(BibliographyError::InvalidBibtexEntry) => Self::InvalidBibtexEntry,
            R::Bibliography(BibliographyError::BibliographyImport(cause)) => {
                Self::BibliographyImport(cause)
            }
        }
    }
}
