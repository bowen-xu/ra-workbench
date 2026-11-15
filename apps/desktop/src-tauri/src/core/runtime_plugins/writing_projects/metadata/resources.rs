use crate::{
    data_interface::plugin_resources::{ColumnCodec, PluginDataError, write_on_connection},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataResourceGrant, DataValue},
    writing_project::Metadata,
};
use rusqlite::Connection;

pub(crate) struct MetadataResource(AcceptedDataResource);

impl MetadataResource {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        AcceptedDataResource::load(
            catalog,
            package,
            "writing_projects",
            "data/project-update.v1.sql",
            DataResourceGrant {
                id: "writing.projects.update".into(),
                version: 1,
                sha256: "37f212b8f293091809c81cadbe1a5ff96996dac7b1dba16443e8412bceb3e78e".into(),
                access: DataAccess::Write,
                parameters: vec![ColumnCodec::String.field(); 3],
                columns: vec![],
            },
        )
        .map(Self)
    }

    pub(crate) fn write(
        &self,
        connection: &Connection,
        id: &str,
        metadata: &Metadata,
    ) -> Result<u64, Box<PluginDataError>> {
        write_on_connection(
            connection,
            &self.0,
            &DataCall {
                id: self.0.grant().id.clone(),
                version: 1,
                arguments: vec![
                    DataValue::Text(id.into()),
                    DataValue::Text(metadata.title.clone()),
                    DataValue::Text(metadata.root_path.clone()),
                ],
            },
        )
    }
}
