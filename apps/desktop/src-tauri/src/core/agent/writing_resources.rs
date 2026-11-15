//! Fixed layout and synchronous repository ports. Queries borrow the caller's
//! SQL snapshot; these functions do not grant plugin consumers database access.
use super::writing_context::WritingContextError;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub(crate) use research_workbench_native_sdk::writing_project::HostResourceFile as WritingResourceFile;

pub(crate) type SourceDirectory = fn(&Path, &str) -> Result<PathBuf, String>;

/// The original Core path locator performs only UUID validation and joining.
pub(crate) fn source_directory(root: &Path, manuscript: &str) -> Result<PathBuf, String> {
    crate::scoped_text_files::legacy::manuscript_source_directory(root, manuscript)
        .map_err(|error| error.to_string())
}

#[derive(Clone, Copy)]
pub(crate) struct WritingResourceSource {
    pub source_directory: SourceDirectory,
    pub is_text_path: fn(&str) -> bool,
    pub list_host_files:
        fn(&Connection, &str) -> Result<Vec<WritingResourceFile>, WritingContextError>,
    pub list_text_paths: fn(&Connection, &str) -> Result<Vec<String>, WritingContextError>,
}

/// Pure path policy and current inventory over the caller's existing snapshot.
/// The resource provider receives no connection or file owner.
pub(crate) trait WritingScopeReader: Send + Sync {
    fn available(&self) -> Result<(), super::context_catalog::ContextCatalogReadError>;
    fn is_text_path(
        &self,
        path: &str,
    ) -> Result<bool, super::context_catalog::ContextCatalogReadError>;
    fn list_text_paths(
        &self,
        db: &Connection,
        manuscript: &str,
    ) -> Result<Vec<String>, super::context_catalog::ContextCatalogReadError>;
}

pub(crate) type OwnedWritingScope = std::sync::Arc<dyn WritingScopeReader>;

pub(crate) async fn native_scope_for_context(
    store: &crate::storage::AppStore,
    record: &research_agent_core::submission::SubmissionRecord,
) -> Option<OwnedWritingScope> {
    let needed = super::context_session::permissions::writing_scope(record).is_some()
        || record.input.context.iter().any(|source| {
            matches!(
                super::context_session::resource_grants::resource(source),
                Some(super::context_session::sources::Resource::Manuscript { .. })
            )
        });
    if !needed {
        return None;
    }
    let binding = store.native_writing_context()?;
    Some(std::sync::Arc::new(binding.scope_resources().await))
}

#[derive(Clone, Copy)]
pub(crate) enum WritingScope<'a> {
    Static(WritingResourceSource),
    Native(&'a dyn WritingScopeReader),
}

pub(crate) fn context_scope<'a>(
    source: Option<WritingResourceSource>,
    reader: Option<&'a dyn super::writing_context::WritingContextReader>,
) -> Option<WritingScope<'a>> {
    reader
        .and_then(|reader| reader.resource_scope())
        .or_else(|| source.map(WritingScope::Static))
}

impl WritingScope<'_> {
    pub(crate) fn require(self) -> Result<Self, super::context_catalog::ContextCatalogReadError> {
        if let Self::Native(source) = self {
            source.available()?;
        }
        Ok(self)
    }

    pub(crate) fn is_text_path(
        self,
        path: &str,
    ) -> Result<bool, super::context_catalog::ContextCatalogReadError> {
        match self {
            Self::Static(source) => Ok((source.is_text_path)(path)),
            Self::Native(source) => source.is_text_path(path),
        }
    }

    pub(crate) fn list_text_paths(
        self,
        db: &Connection,
        manuscript: &str,
    ) -> Result<Vec<String>, super::context_catalog::ContextCatalogReadError> {
        match self {
            Self::Static(source) => (source.list_text_paths)(db, manuscript)
                .map_err(|_| super::context_catalog::ContextCatalogReadError::Unavailable),
            Self::Native(source) => source.list_text_paths(db, manuscript),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_sources;
