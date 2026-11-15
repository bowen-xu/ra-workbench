//! Fixed domain candidate queries used by the Core context directory. These
//! internal repository operations borrow the caller's existing connection;
//! no connection owner, transaction or arbitrary query is exposed to policy.
use rusqlite::Connection;
use serde::Serialize;
pub(crate) type ContextCatalogEntries = Vec<(String, String)>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextCatalogError {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "The finite Core catalog callback retains its unavailable result for test and boundary adapters."
        )
    )]
    Unavailable,
}
type Query = fn(&Connection, &str) -> Result<ContextCatalogEntries, ContextCatalogError>;
#[derive(Clone, Copy, Default)]
pub(crate) struct ContextCatalogSource {
    pub search_papers: Option<Query>,
    pub list_pdf_attachments: Option<Query>,
    pub search_manuscripts: Option<Query>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCatalogFailure {
    pub code: String,
    pub message: &'static str,
    pub package_key: String,
    pub phase: &'static str,
    pub safe_cause: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqlite_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqlite_extended_code: Option<i32>,
}

impl ContextCatalogFailure {
    pub(crate) fn diagnostic(&self) -> String {
        let mut text = format!(
            "{}: {}; packageKey={}; phase={}; cause={}",
            self.code, self.message, self.package_key, self.phase, self.safe_cause
        );
        if let Some(id) = &self.resource_id {
            text.push_str(&format!("; resourceId={id}"));
        }
        if let Some(version) = self.resource_version {
            text.push_str(&format!("; resourceVersion={version}"));
        }
        if let Some(code) = &self.sqlite_code {
            text.push_str(&format!("; sqliteCode={code}"));
        }
        if let Some(code) = self.sqlite_extended_code {
            text.push_str(&format!("; sqliteExtendedCode={code}"));
        }
        text
    }
}

impl std::fmt::Display for ContextCatalogFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.diagnostic())
    }
}
impl std::error::Error for ContextCatalogFailure {}

#[derive(Clone, Debug)]
pub(crate) enum ContextCatalogReadError {
    Unavailable,
    SourceUnavailable,
    Native(Box<ContextCatalogFailure>),
}

/// Only the literature inventory varies; Core keeps grouping and permissions.
pub(crate) trait PaperCatalogReader {
    fn search_papers(
        &self,
        db: &Connection,
        query: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError>;
    fn list_pdf_attachments(
        &self,
        db: &Connection,
        paper: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError>;
}

/// A directory lookup borrows the caller's snapshot. It does not authorize a
/// manuscript or freeze its files for later tools.
pub(crate) trait ManuscriptCatalogReader {
    fn search_manuscripts(
        &self,
        db: &Connection,
        query: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError>;
    fn list_text_paths(
        &self,
        db: &Connection,
        manuscript: &str,
    ) -> Result<Vec<String>, ContextCatalogReadError>;
}

pub(crate) struct StaticManuscriptCatalog {
    pub catalog: ContextCatalogSource,
    pub resources: Option<super::writing_resources::WritingResourceSource>,
}

impl ManuscriptCatalogReader for StaticManuscriptCatalog {
    fn search_manuscripts(
        &self,
        db: &Connection,
        query: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError> {
        (self
            .catalog
            .search_manuscripts
            .ok_or(ContextCatalogReadError::Unavailable)?)(db, query)
        .map_err(|_| ContextCatalogReadError::Unavailable)
    }
    fn list_text_paths(
        &self,
        db: &Connection,
        manuscript: &str,
    ) -> Result<Vec<String>, ContextCatalogReadError> {
        (self
            .resources
            .ok_or(ContextCatalogReadError::Unavailable)?
            .list_text_paths)(db, manuscript)
        .map_err(|_| ContextCatalogReadError::Unavailable)
    }
}

impl PaperCatalogReader for ContextCatalogSource {
    fn search_papers(
        &self,
        db: &Connection,
        query: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError> {
        (self
            .search_papers
            .ok_or(ContextCatalogReadError::SourceUnavailable)?)(db, query)
        .map_err(|_| ContextCatalogReadError::Unavailable)
    }
    fn list_pdf_attachments(
        &self,
        db: &Connection,
        paper: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError> {
        (self
            .list_pdf_attachments
            .ok_or(ContextCatalogReadError::SourceUnavailable)?)(db, paper)
        .map_err(|_| ContextCatalogReadError::Unavailable)
    }
}
