//! Two package-owned inventory reads; Core retains directory and grant semantics.
use crate::{
    agent::context_catalog::{
        ContextCatalogEntries, ContextCatalogFailure, ContextCatalogReadError, PaperCatalogReader,
    },
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue,
};
use rusqlite::Connection;

const MESSAGE: &str = "文献上下文目录来源不可用。";
const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};

pub(crate) struct ContextCatalogResources {
    papers: AcceptedDataResource,
    attachments: AcceptedDataResource,
}

impl ContextCatalogResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, ContextCatalogReadError> {
        let load = |name: &str, sha: &str| {
            let id = format!("literature.context-catalog.{name}");
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/context-catalog-{name}.v1.sql"),
                DataResourceGrant {
                    id: id.clone(),
                    version: 1,
                    sha256: sha.into(),
                    access: DataAccess::Read,
                    parameters: vec![TEXT],
                    columns: vec![TEXT, TEXT],
                },
            )
            .map_err(|error| resource_failure(error, &package.package_key, id))
        };
        Ok(Self {
            papers: load(
                "papers",
                "248da1be21fc97b914c0631a2fe59353ac80849471fe7e8d6072ff0d1ed5e7f6",
            )?,
            attachments: load(
                "pdf-attachments",
                "e9bffd3fa0ece61539bd06341b6a659725c686f43733074fe81dcbc7f4c13be4",
            )?,
        })
    }

    fn pairs(
        resource: &AcceptedDataResource,
        db: &Connection,
        key: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError> {
        let rows = read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(key.into())],
            },
            RowPolicy::All,
            &[ColumnCodec::String, ColumnCodec::String],
        )
        .map_err(|error| data_failure(*error))?;
        rows.rows
            .into_iter()
            .map(|row| match row.as_slice() {
                [DataValue::Text(id), DataValue::Text(label)] => Ok((id.clone(), label.clone())),
                _ => Err(ContextCatalogReadError::Native(Box::new(
                    ContextCatalogFailure {
                        code: "plugin_data_result_incompatible".into(),
                        message: MESSAGE,
                        package_key: resource.package_key().into(),
                        phase: "read",
                        safe_cause: "context_catalog_data_shape".into(),
                        resource_id: Some(resource.grant().id.clone()),
                        resource_version: Some(1),
                        sqlite_code: None,
                        sqlite_extended_code: None,
                    },
                ))),
            })
            .collect()
    }
}

impl PaperCatalogReader for ContextCatalogResources {
    fn search_papers(
        &self,
        db: &Connection,
        query: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError> {
        Self::pairs(&self.papers, db, query)
    }
    fn list_pdf_attachments(
        &self,
        db: &Connection,
        paper: &str,
    ) -> Result<ContextCatalogEntries, ContextCatalogReadError> {
        Self::pairs(&self.attachments, db, paper)
    }
}

pub(crate) fn admission_failure(code: &str, package_key: &str) -> ContextCatalogReadError {
    ContextCatalogReadError::Native(Box::new(ContextCatalogFailure {
        code: code.into(),
        message: MESSAGE,
        package_key: package_key.into(),
        phase: "admission",
        safe_cause: code.into(),
        resource_id: None,
        resource_version: None,
        sqlite_code: None,
        sqlite_extended_code: None,
    }))
}

fn resource_failure(
    error: RuntimePluginError,
    package_key: &str,
    id: String,
) -> ContextCatalogReadError {
    ContextCatalogReadError::Native(Box::new(ContextCatalogFailure {
        code: error.code.into(),
        message: MESSAGE,
        package_key: package_key.into(),
        phase: "resource",
        safe_cause: error.code.into(),
        resource_id: Some(id),
        resource_version: Some(1),
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
