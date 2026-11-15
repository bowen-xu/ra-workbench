use super::super::super::{metadata::resources::MetadataResource, resources::SessionResources};
use super::super::{changes::ChangeResources, resources::FileResources};
use super::probe::{Observation, Relations};
use crate::{
    data_interface::plugin_resources::{ColumnCodec, PluginDataError, write_in},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{DataAccess, DataCall, DataResourceGrant, DataValue};
use rusqlite::Transaction;

pub(crate) struct DirectoryResources {
    pub(super) changes: ChangeResources,
    pub(super) metadata: MetadataResource,
    pub(super) sessions: SessionResources,
    pub(super) manuscript: AcceptedDataResource,
    pub(super) files: AcceptedDataResource,
    drafts: AcceptedDataResource,
    rename_draft: AcceptedDataResource,
    delete_draft: AcceptedDataResource,
    touch: AcceptedDataResource,
}
impl DirectoryResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
        base: &FileResources,
    ) -> Result<Self, RuntimePluginError> {
        let text = ColumnCodec::String.field();
        let load = |name: &str, hash: &str, access, parameters, columns| {
            AcceptedDataResource::load(
                catalog,
                package,
                "writing_projects",
                &format!("data/project-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("writing.projects.{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            changes: ChangeResources::load(catalog, package)?,
            metadata: MetadataResource::load(catalog, package)?,
            sessions: SessionResources::load(catalog, package)?,
            manuscript: base.projects.get.clone(),
            files: base.files.clone(),
            drafts: load(
                "directory-draft-paths",
                "59e524c03550fbbb0f10c5cfe94f8cd646bd3aebd752a70a281a8c9501ceb9fa",
                DataAccess::Read,
                vec![text],
                vec![text],
            )?,
            rename_draft: load(
                "directory-draft-rename",
                "b0b2367d8f335f48edfd9606045ca8be878f9fa6cec9273168dc1bc4951d7994",
                DataAccess::Write,
                vec![text; 3],
                vec![],
            )?,
            delete_draft: load(
                "draft-delete",
                "6d48fb5e85dde34efb4ebf6fd2ecad7a9f69616992f26ba0965b03e62ac07acb",
                DataAccess::Write,
                vec![text; 2],
                vec![],
            )?,
            touch: load(
                "touch",
                "96589127b930fbaef08504cf202a7f5106694453e23c28e5df2c19239ad37989",
                DataAccess::Write,
                vec![text],
                vec![],
            )?,
        })
    }
    pub(super) fn capture(&self, tx: &Transaction<'_>, id: &str) -> Relations {
        Relations {
            manuscript: Observation::capture(tx, &self.manuscript, id),
            files: Observation::capture(tx, &self.files, id),
            drafts: Observation::capture(tx, &self.drafts, id),
            session: Observation::capture(tx, &self.sessions.read, id),
        }
    }
    fn write(
        &self,
        tx: &Transaction<'_>,
        resource: &AcceptedDataResource,
        values: &[&str],
    ) -> Result<(), Box<PluginDataError>> {
        write_in(
            tx,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: values
                    .iter()
                    .map(|value| DataValue::Text((*value).into()))
                    .collect(),
            },
        )
        .map(|_| ())
    }
    pub(super) fn rename_draft(
        &self,
        tx: &Transaction<'_>,
        id: &str,
        from: &str,
        to: &str,
    ) -> Result<(), Box<PluginDataError>> {
        self.write(tx, &self.rename_draft, &[id, from, to])
    }
    pub(super) fn delete_draft(
        &self,
        tx: &Transaction<'_>,
        id: &str,
        path: &str,
    ) -> Result<(), Box<PluginDataError>> {
        self.write(tx, &self.delete_draft, &[id, path])
    }
    pub(super) fn touch(&self, tx: &Transaction<'_>, id: &str) -> Result<(), Box<PluginDataError>> {
        self.write(tx, &self.touch, &[id])
    }
}
