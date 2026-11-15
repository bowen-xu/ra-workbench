//! A request borrows the original inventory and immutable package path policy.
use super::{
    Binding,
    catalog::{ManuscriptCatalogResources, failure},
};
use crate::{
    agent::{
        context_catalog::{ContextCatalogReadError, ManuscriptCatalogReader},
        writing_resources::WritingScopeReader,
    },
    runtime_plugins::{RuntimePluginCatalog, RuntimePluginPackage, data::load_text},
};
use research_workbench_native_sdk::writing_file::FileKindPolicyV1;
use rusqlite::Connection;

pub(crate) struct ScopeResources {
    catalog: ManuscriptCatalogResources,
    policy: FileKindPolicyV1,
}

impl Binding {
    pub(crate) async fn scope_resources(&self) -> Result<ScopeResources, ContextCatalogReadError> {
        self.load_context_resources(ScopeResources::load).await
    }
}
impl ScopeResources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, ContextCatalogReadError> {
        let id = "writing.projects.file-kinds";
        let invalid = |code| failure(code, &package.package_key, "resource", Some(id.into()));
        let text = load_text(
            catalog,
            package,
            "writing_projects",
            "data/project-file-kinds.v1.json",
            "428e510573de752a11a0839b75a5f6c1f421d22013672f02dd81ca28f01708ea",
        )
        .map_err(|error| invalid(error.code))?;
        let policy: FileKindPolicyV1 =
            serde_json::from_str(&text).map_err(|_| invalid("writing_file_kind_policy_invalid"))?;
        if policy.format_version != 1 {
            return Err(invalid("writing_file_kind_policy_version_incompatible"));
        }
        Ok(Self {
            catalog: ManuscriptCatalogResources::load(catalog, package)?,
            policy,
        })
    }
}
impl WritingScopeReader for ScopeResources {
    fn available(&self) -> Result<(), ContextCatalogReadError> {
        Ok(())
    }
    fn is_text_path(&self, path: &str) -> Result<bool, ContextCatalogReadError> {
        Ok(self.policy.is_text_path(path))
    }
    fn list_text_paths(
        &self,
        db: &Connection,
        manuscript: &str,
    ) -> Result<Vec<String>, ContextCatalogReadError> {
        self.catalog.list_text_paths(db, manuscript)
    }
}

/// Admission happens outside the DB guard. Its result is consumed only where
/// the original validation first required the writing source.
impl WritingScopeReader for Result<ScopeResources, ContextCatalogReadError> {
    fn available(&self) -> Result<(), ContextCatalogReadError> {
        self.as_ref().map(|_| ()).map_err(Clone::clone)
    }
    fn is_text_path(&self, path: &str) -> Result<bool, ContextCatalogReadError> {
        self.as_ref().map_err(Clone::clone)?.is_text_path(path)
    }
    fn list_text_paths(
        &self,
        db: &Connection,
        manuscript: &str,
    ) -> Result<Vec<String>, ContextCatalogReadError> {
        self.as_ref()
            .map_err(Clone::clone)?
            .list_text_paths(db, manuscript)
    }
}

impl From<Box<crate::agent::context_catalog::ContextCatalogFailure>>
    for crate::agent::writing_submission::WritingSubmissionError
{
    fn from(error: Box<crate::agent::context_catalog::ContextCatalogFailure>) -> Self {
        let mut native = crate::runtime_plugins::native::NativeHostError::for_profile(
            crate::runtime_plugins::native::NativeProfile::WritingProjects,
            &error.code,
            format!(
                "{} phase={} cause={}",
                error.message, error.phase, error.safe_cause
            ),
        )
        .package(&error.package_key);
        if let (Some(id), Some(version)) = (error.resource_id, error.resource_version) {
            native.data_error = Some(Box::new(
                crate::data_interface::plugin_resources::PluginDataError {
                    code: "writing_scope_source_unavailable",
                    message: error.message,
                    package_key: error.package_key,
                    resource_id: id,
                    resource_version: version,
                    phase: error.phase,
                    safe_cause: error.safe_cause,
                    sqlite_code: error.sqlite_code,
                    sqlite_extended_code: error.sqlite_extended_code,
                    original_sql_failure: None,
                },
            ));
        }
        native.into()
    }
}
