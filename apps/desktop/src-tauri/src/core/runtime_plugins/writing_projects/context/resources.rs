//! Original context reads, accepted under the same exact package identity.
use super::super::files::resources::FileResources;
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataResourceGrant, DataRows, DataValue,
};
use rusqlite::Connection;

pub(super) struct Resources {
    pub files: FileResources,
    current: AcceptedDataResource,
}
impl Resources {
    pub fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let text = ColumnCodec::String.field();
        Ok(Self {
            files: FileResources::load(catalog, package)?,
            current: AcceptedDataResource::load(
                catalog,
                package,
                "writing_projects",
                "data/project-context-current-file.v1.sql",
                DataResourceGrant {
                    id: "writing.projects.context-current-file".into(),
                    version: 1,
                    sha256: "e401b68f5950d6d8421c3049c9a85848992ebb278a4e4eada51d6794d79f2288"
                        .into(),
                    access: DataAccess::Read,
                    parameters: vec![text; 2],
                    columns: vec![text],
                },
            )?,
        })
    }
    pub fn current(
        &self,
        db: &Connection,
        manuscript: &str,
        path: &str,
    ) -> Result<DataRows, Box<PluginDataError>> {
        read_on_connection(
            db,
            &self.current,
            &DataCall {
                id: self.current.grant().id.clone(),
                version: 1,
                arguments: vec![
                    DataValue::Text(manuscript.into()),
                    DataValue::Text(path.into()),
                ],
            },
            RowPolicy::FirstRequired,
            &[ColumnCodec::String],
        )
    }
}
