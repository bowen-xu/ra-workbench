//! Fixed formal-history adapter for the original authorized Agent reader.
//! The caller owns both database guards. Native waits start after they are released.
use super::{domain::Domain, repository, resources::Sql, synctex};
use crate::{
    agent::writing_error::WritingError,
    runtime_plugins::{
        native::{NativeResult, ProjectOperation},
        writing_projects::context::Binding,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_compile::{CompileRun, SynctexPdfLocation};
use rusqlite::Connection;

pub(crate) struct Feedback {
    domain: Domain,
    sql: Sql,
}

impl Feedback {
    pub(crate) fn new(binding: Binding, runtime: tokio::runtime::Handle) -> NativeResult<Self> {
        let domain = Domain::new(binding, runtime);
        let sql = domain.resources()?;
        Ok(Self { domain, sql })
    }

    pub(crate) fn latest(
        &self,
        db: &Connection,
        manuscript_id: &str,
    ) -> Result<Option<CompileRun>, WritingError> {
        repository::latest_run(&self.sql, db, manuscript_id)
    }

    pub(crate) fn latest_pdf(
        &self,
        db: &Connection,
        manuscript_id: &str,
    ) -> Result<Option<CompileRun>, WritingError> {
        repository::latest_pdf_run(&self.sql, db, manuscript_id)
    }

    /// Transfer no database ownership. Retain the existing gate for the complete
    /// native projection or query, including the original second context read.
    pub(crate) fn enter(&self) -> NativeResult<ProjectOperation> {
        self.domain.operation()
    }

    pub(crate) fn project(&self, run: CompileRun) -> NativeResult<CompileRun> {
        self.domain.run(run)
    }

    pub(crate) fn forward(
        &self,
        store: &AppStore,
        run_id: &str,
        path: &str,
        line: u32,
        column: u32,
    ) -> NativeResult<Vec<SynctexPdfLocation>> {
        synctex::forward(store, &self.domain, &self.sql, run_id, path, line, column)
    }
}
