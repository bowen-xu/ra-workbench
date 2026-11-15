//! Exact original borrowed-connection startup reads. No transaction or file owner.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataResourceGrant, DataRows, DataValue},
    writing_project::CitationMetadataKind,
};
use rusqlite::Connection;

#[derive(Clone)]
pub(super) struct Resources {
    resource: AcceptedDataResource,
    kind: CitationMetadataKind,
}

impl Resources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
        kind: CitationMetadataKind,
    ) -> Result<Self, RuntimePluginError> {
        let (name, hash) = match kind {
            CitationMetadataKind::Bindings => (
                "citation-bindings",
                "ad52c976fde4ee44b2acee01cec735854ce26c71a4f03b2bcd2fe6d12aace4f5",
            ),
            CitationMetadataKind::BibliographyStatus => (
                "bibliography-status",
                "84b75cc1f3f6657fa535088c8a11b40036bf0f069e4d8eb251fb83ec1afacadc",
            ),
        };
        let resource = AcceptedDataResource::load(
            catalog,
            package,
            "writing_projects",
            &format!("data/project-{name}.v1.sql"),
            DataResourceGrant {
                id: format!("writing.projects.{name}"),
                version: 1,
                sha256: hash.into(),
                access: DataAccess::Read,
                parameters: vec![ColumnCodec::String.field()],
                columns: Self::columns(kind)
                    .iter()
                    .map(|codec| codec.field())
                    .collect(),
            },
        )?;
        Ok(Self { resource, kind })
    }

    fn columns(kind: CitationMetadataKind) -> &'static [ColumnCodec] {
        match kind {
            CitationMetadataKind::Bindings => &[ColumnCodec::String; 7],
            CitationMetadataKind::BibliographyStatus => &[
                ColumnCodec::String,
                ColumnCodec::OptionalString,
                ColumnCodec::OptionalString,
                ColumnCodec::String,
            ],
        }
    }

    pub(super) fn read(&self, db: &Connection, id: &str) -> Result<DataRows, Box<PluginDataError>> {
        let policy = match self.kind {
            CitationMetadataKind::Bindings => RowPolicy::All,
            CitationMetadataKind::BibliographyStatus => RowPolicy::FirstOptional,
        };
        read_on_connection(
            db,
            &self.resource,
            &DataCall {
                id: self.resource.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(id.into())],
            },
            policy,
            Self::columns(self.kind),
        )
    }

    pub(super) fn recheck(
        &self,
        db: &Connection,
        id: &str,
        captured: &DataRows,
    ) -> Result<(), Box<PluginDataError>> {
        if self.read(db, id)? != *captured {
            return Err(Box::new(PluginDataError {
                code: "plugin_data_snapshot_changed",
                message: "插件计算所依据的数据已变化；请重新读取。",
                package_key: self.resource.package_key().into(),
                resource_id: self.resource.grant().id.clone(),
                resource_version: 1,
                phase: "recheck",
                safe_cause: "writing_citation_metadata_read_set_changed".into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            }));
        }
        Ok(())
    }
}
