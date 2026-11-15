//! Exact original checkpoint statements and original row decoding.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataResourceGrant, DataRows, DataValue},
    writing_checkpoint::CheckpointRecord,
};
use rusqlite::Connection;
pub(crate) struct Resources {
    read: AcceptedDataResource,
    paths: AcceptedDataResource,
    write: AcceptedDataResource,
}
impl Resources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let text = ColumnCodec::String.field();
        let integer = ColumnCodec::U64.field();
        let load = |name: &str, hash: &str, access, parameters, columns| {
            AcceptedDataResource::load(
                catalog,
                package,
                "writing_projects",
                &format!("data/project-checkpoint-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("writing.projects.checkpoint-{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            read: load(
                "read",
                "6154b46d09a6251d55237ebc6374d54eb8df112ae8afd2e643b30bdf6d366142",
                DataAccess::Read,
                vec![text; 2],
                vec![integer, text],
            )?,
            paths: load(
                "paths",
                "b699fbe2ecd96af6ab2d700b24c8dc7924c7a60ebbe55ad399adfd19cdd779cb",
                DataAccess::Read,
                vec![text],
                vec![text],
            )?,
            write: load(
                "upsert",
                "c9c9ae1d372a7c4d7aa606849c9b04ba3cfe00582594187ff42bc738659722e0",
                DataAccess::Write,
                vec![text, text, integer, text],
                vec![],
            )?,
        })
    }
    fn call(resource: &AcceptedDataResource, values: Vec<DataValue>) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: values,
        }
    }
    pub(crate) fn read(
        &self,
        db: &Connection,
        id: &str,
        path: &str,
    ) -> Result<DataRows, Box<PluginDataError>> {
        read_on_connection(
            db,
            &self.read,
            &Self::call(
                &self.read,
                vec![DataValue::Text(id.into()), DataValue::Text(path.into())],
            ),
            RowPolicy::FirstOptional,
            &[ColumnCodec::U64, ColumnCodec::String],
        )
    }
    pub(super) fn paths(
        &self,
        db: &Connection,
        id: &str,
    ) -> Result<DataRows, Box<PluginDataError>> {
        read_on_connection(
            db,
            &self.paths,
            &Self::call(&self.paths, vec![DataValue::Text(id.into())]),
            RowPolicy::All,
            &[ColumnCodec::String],
        )
    }
    pub(super) fn write(
        &self,
        db: &Connection,
        record: &CheckpointRecord,
        json: String,
    ) -> Result<(), Box<PluginDataError>> {
        write_on_connection(
            db,
            &self.write,
            &Self::call(
                &self.write,
                vec![
                    DataValue::Text(record.checkpoint.manuscript_id.clone()),
                    DataValue::Text(record.checkpoint.relative_path.clone()),
                    DataValue::Integer(record.revision as i64),
                    DataValue::Text(json),
                ],
            ),
        )
        .map(|_| ())
    }
}
