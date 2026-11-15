//! Original autocommit index changes and shared manuscript metadata grant.
use crate::{
    data_interface::plugin_resources::{ColumnCodec, PluginDataError, write_on_connection},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
        writing_projects::metadata::resources::MetadataResource,
    },
};
use research_workbench_native_sdk::data::{DataAccess, DataCall, DataResourceGrant, DataValue};
use rusqlite::Connection;

pub(crate) struct ChangeResources {
    rename: AcceptedDataResource,
    delete: AcceptedDataResource,
    pub(super) metadata: MetadataResource,
}

impl ChangeResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, hash: &str, count| {
            AcceptedDataResource::load(
                catalog,
                package,
                "writing_projects",
                &format!("data/project-file-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("writing.projects.file-{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access: DataAccess::Write,
                    parameters: vec![ColumnCodec::String.field(); count],
                    columns: vec![],
                },
            )
        };
        Ok(Self {
            rename: load(
                "rename",
                "4ff5967a7eca0e33fe1ea3c1b3b90e4c4f42221bd7c573400bf9ff3fe735f2a4",
                3,
            )?,
            delete: load(
                "delete",
                "bd94eb1cb3913f8b0f1c2c105a6ac1d5e0b7fb00bf1b682b9e6c7bb65b1f1716",
                2,
            )?,
            metadata: MetadataResource::load(catalog, package)?,
        })
    }
    fn call(resource: &AcceptedDataResource, values: &[&str]) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: values
                .iter()
                .map(|value| DataValue::Text((*value).into()))
                .collect(),
        }
    }
    pub(crate) fn rename(
        &self,
        db: &Connection,
        id: &str,
        from: &str,
        to: &str,
    ) -> Result<u64, Box<PluginDataError>> {
        write_on_connection(db, &self.rename, &Self::call(&self.rename, &[id, from, to]))
    }
    pub(crate) fn delete(
        &self,
        db: &Connection,
        id: &str,
        path: &str,
    ) -> Result<(), Box<PluginDataError>> {
        write_on_connection(db, &self.delete, &Self::call(&self.delete, &[id, path])).map(|_| ())
    }
}
