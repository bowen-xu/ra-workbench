//! Original closed file-index port and its admitted runtime counterpart.
use super::WritingWorkspaceSource;
use crate::{
    agent::{
        writing_context::WritingContextError,
        writing_submission::{WritingSubmissionError, workspace},
    },
    runtime_plugins::{
        native::NativeHostError, writing_projects::files::agent_index::PreparedIndex,
    },
};
use research_agent_core::workspace::FileVersion;
use rusqlite::Connection;
use uuid::Uuid;

pub(crate) enum IndexFailure {
    Original(WritingContextError),
    Source(Box<NativeHostError>),
}
impl IndexFailure {
    pub(crate) fn map(
        self,
        original: impl FnOnce(WritingContextError) -> workspace::Error,
    ) -> workspace::Error {
        match self {
            Self::Original(cause) => original(cause),
            Self::Source(cause) => WritingSubmissionError::from(cause).into(),
        }
    }
}

#[derive(Clone)]
pub(crate) enum FileIndex {
    Static(Option<WritingWorkspaceSource>),
    Runtime(PreparedIndex),
}
impl FileIndex {
    fn source(
        source: &Option<WritingWorkspaceSource>,
    ) -> Result<WritingWorkspaceSource, IndexFailure> {
        source.ok_or(IndexFailure::Original(WritingContextError::Unavailable))
    }
    pub(crate) fn check_unregistered(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
    ) -> Result<(), IndexFailure> {
        match self {
            Self::Static(source) => {
                (Self::source(source)?.check_unregistered)(db, manuscript, path)
                    .map_err(IndexFailure::Original)
            }
            Self::Runtime(source) => source.check_unregistered(db, manuscript, path),
        }
    }
    pub(crate) fn check_registered(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        before: &FileVersion,
    ) -> Result<(), IndexFailure> {
        match self {
            Self::Static(source) => {
                (Self::source(source)?.check_registered)(db, manuscript, path, before)
                    .map_err(IndexFailure::Original)
            }
            Self::Runtime(source) => source.check_registered(db, manuscript, path, before),
        }
    }
    pub(crate) fn register_created(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        actual: &FileVersion,
    ) -> Result<(), IndexFailure> {
        match self {
            Self::Static(source) => {
                (Self::source(source)?.register_created)(db, manuscript, path, actual)
                    .map_err(IndexFailure::Original)
            }
            Self::Runtime(source) => source.register_created(db, manuscript, path, actual),
        }
    }
    pub(crate) fn update_registered(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        before: &FileVersion,
        actual: Option<&FileVersion>,
    ) -> Result<(), IndexFailure> {
        match self {
            Self::Static(source) => {
                (Self::source(source)?.update_registered)(db, manuscript, path, before, actual)
                    .map_err(IndexFailure::Original)
            }
            Self::Runtime(source) => source.update_registered(db, manuscript, path, before, actual),
        }
    }
}
