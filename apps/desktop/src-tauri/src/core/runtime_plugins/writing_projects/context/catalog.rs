//! Original directory statements, admitted before taking the caller's DB guard.
use super::Binding;
use crate::{
    agent::context_catalog::{
        ContextCatalogEntries, ContextCatalogFailure, ContextCatalogReadError,
        ManuscriptCatalogReader,
    },
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginPackage,
        data::AcceptedDataResource,
        native::{NativeProfile, package},
    },
};
use research_workbench_native_sdk::data::{DataAccess, DataCall, DataResourceGrant, DataValue};
use rusqlite::Connection;
use std::sync::Arc;

const MESSAGE: &str = "写作上下文目录来源不可用。";

pub(crate) struct ManuscriptCatalogResources {
    manuscripts: AcceptedDataResource,
    paths: AcceptedDataResource,
}

impl Binding {
    pub(crate) async fn context_catalog_resources(
        &self,
    ) -> Result<ManuscriptCatalogResources, ContextCatalogReadError> {
        self.load_context_resources(ManuscriptCatalogResources::load)
            .await
    }

    pub(super) async fn load_context_resources<T: Send + 'static>(
        &self,
        load: fn(
            &RuntimePluginCatalog,
            &RuntimePluginPackage,
        ) -> Result<T, ContextCatalogReadError>,
    ) -> Result<T, ContextCatalogReadError> {
        let _operation = self
            .service
            .project_operation(&self.package_key)
            .await
            .map_err(|error| failure(&error.code, &self.package_key, "admission", None))?;
        let catalog = Arc::clone(&self.catalog);
        let key = self.package_key.clone();
        tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &key, NativeProfile::WritingProjects)
                .map_err(|error| failure(&error.code, &key, "admission", None))?;
            load(&catalog, &package)
        })
        .await
        .map_err(|_| {
            failure(
                "plugin_host_accept_failed",
                &self.package_key,
                "admission",
                None,
            )
        })?
    }
}

impl ManuscriptCatalogResources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, ContextCatalogReadError> {
        let load = |name: &str, hash: &str, columns: usize| {
            let id = format!("writing.projects.context-{name}");
            AcceptedDataResource::load(
                catalog,
                package,
                "writing_projects",
                &format!("data/project-context-{name}.v1.sql"),
                DataResourceGrant {
                    id: id.clone(),
                    version: 1,
                    sha256: hash.into(),
                    access: DataAccess::Read,
                    parameters: vec![ColumnCodec::String.field()],
                    columns: vec![ColumnCodec::String.field(); columns],
                },
            )
            .map_err(|error| failure(error.code, &package.package_key, "resource", Some(id)))
        };
        Ok(Self {
            manuscripts: load(
                "catalog",
                "c77e7be91f8b3000f557c2b15f3fb9040d774529a544ec16c4375ba5bf8c1e79",
                2,
            )?,
            paths: load(
                "text-paths",
                "cb8b873ae172e0f1e03cac1c938aa386d2620ec01dbd6b4efb19150d2080c748",
                1,
            )?,
        })
    }
    fn read(
        resource: &AcceptedDataResource,
        db: &Connection,
        key: &str,
        columns: usize,
    ) -> Result<Vec<Vec<DataValue>>, ContextCatalogReadError> {
        read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(key.into())],
            },
            RowPolicy::All,
            &vec![ColumnCodec::String; columns],
        )
        .map(|rows| rows.rows)
        .map_err(|error| data_failure(*error))
    }
    fn shape(resource: &AcceptedDataResource) -> ContextCatalogReadError {
        failure(
            "plugin_data_result_incompatible",
            resource.package_key(),
            "read",
            Some(resource.grant().id.clone()),
        )
    }
}

impl ManuscriptCatalogReader for ManuscriptCatalogResources {
    fn search_manuscripts(
        &self,
        db: &Connection,
        query: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError> {
        Self::read(&self.manuscripts, db, query, 2)?
            .into_iter()
            .map(|row| match row.as_slice() {
                [DataValue::Text(id), DataValue::Text(title)] => Ok((id.clone(), title.clone())),
                _ => Err(Self::shape(&self.manuscripts)),
            })
            .collect()
    }
    fn list_text_paths(
        &self,
        db: &Connection,
        manuscript: &str,
    ) -> Result<Vec<String>, ContextCatalogReadError> {
        Self::read(&self.paths, db, manuscript, 1)?
            .into_iter()
            .map(|row| match row.as_slice() {
                [DataValue::Text(path)] => Ok(path.clone()),
                _ => Err(Self::shape(&self.paths)),
            })
            .collect()
    }
}

pub(super) fn failure(
    code: &str,
    key: &str,
    phase: &'static str,
    resource_id: Option<String>,
) -> ContextCatalogReadError {
    let resource_version = resource_id.as_ref().map(|_| 1);
    ContextCatalogReadError::Native(Box::new(ContextCatalogFailure {
        code: code.into(),
        message: MESSAGE,
        package_key: key.into(),
        phase,
        safe_cause: code.into(),
        resource_id,
        resource_version,
        sqlite_code: None,
        sqlite_extended_code: None,
    }))
}
fn data_failure(error: PluginDataError) -> ContextCatalogReadError {
    ContextCatalogReadError::Native(Box::new(ContextCatalogFailure {
        code: error.code.into(),
        message: MESSAGE,
        package_key: error.package_key,
        phase: "read",
        safe_cause: error.safe_cause,
        resource_id: Some(error.resource_id),
        resource_version: Some(error.resource_version),
        sqlite_code: error.sqlite_code,
        sqlite_extended_code: error.sqlite_extended_code,
    }))
}
