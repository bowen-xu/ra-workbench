use super::StatusAction;
use crate::{
    data_interface::plugin_resources::{ColumnCodec, PluginDataError, write_on_connection},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{DataAccess, DataCall, DataResourceGrant, DataValue};
use rusqlite::Connection;

pub(super) struct StatusResource(AcceptedDataResource);

impl StatusResource {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
        action: StatusAction,
    ) -> Result<Self, RuntimePluginError> {
        let (name, hash) = match action {
            StatusAction::Archive => (
                "archive",
                "a0266a15f49b18a792ab47e09295816d09f454434f48368166dfb51df0475af0",
            ),
            StatusAction::Restore => (
                "restore",
                "c04890965039664cfa7b65fcfb9b21d29087835c4ca3f10f7a231758c86f2f9c",
            ),
        };
        AcceptedDataResource::load(
            catalog,
            package,
            "writing_projects",
            &format!("data/project-{name}.v1.sql"),
            DataResourceGrant {
                id: format!("writing.projects.{name}"),
                version: 1,
                sha256: hash.into(),
                access: DataAccess::Write,
                parameters: vec![ColumnCodec::String.field()],
                columns: vec![],
            },
        )
        .map(Self)
    }

    pub(super) fn write(
        &self,
        connection: &Connection,
        id: &str,
    ) -> Result<u64, Box<PluginDataError>> {
        write_on_connection(
            connection,
            &self.0,
            &DataCall {
                id: self.0.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(id.into())],
            },
        )
    }
}
