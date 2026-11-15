use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_in,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
        native::NativeResult,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataResourceGrant, DataRows, DataValue,
};
use rusqlite::{Connection, Transaction};

pub(crate) struct DeletionResources {
    active: AcceptedDataResource,
    delete: AcceptedDataResource,
}
impl DeletionResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, hash: &str, access, columns| {
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
                    parameters: vec![ColumnCodec::String.field()],
                    columns,
                },
            )
        };
        Ok(Self {
            active: load(
                "active-compile",
                "a3a4abb8545011ef2269de572163c5bb35b9ffa2bfa606fbb3ff0d7d4f5b1315",
                DataAccess::Read,
                vec![ColumnCodec::Bool.field()],
            )?,
            delete: load(
                "delete",
                "700b4e488d89df911fb0ea5851045f3e3e241e8905424c855135642139ffc1b3",
                DataAccess::Write,
                vec![],
            )?,
        })
    }
    fn call(&self, resource: &AcceptedDataResource, id: &str) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: vec![DataValue::Text(id.into())],
        }
    }
    pub(super) fn active(
        &self,
        db: &Connection,
        id: &str,
    ) -> Result<DataRows, Box<PluginDataError>> {
        read_on_connection(
            db,
            &self.active,
            &self.call(&self.active, id),
            RowPolicy::FirstRequired,
            &[ColumnCodec::Bool],
        )
    }
    pub(super) fn recheck(
        &self,
        db: &Connection,
        id: &str,
        captured: &DataRows,
    ) -> NativeResult<()> {
        let current = self
            .active(db, id)
            .map_err(super::super::super::data_failure)?;
        if current != *captured {
            return Err(super::super::super::data_failure(Box::new(
                PluginDataError {
                    code: "plugin_data_snapshot_changed",
                    message: "稿件活动编译状态已变化；本次尚未删除。",
                    package_key: self.active.package_key().into(),
                    resource_id: self.active.grant().id.clone(),
                    resource_version: 1,
                    phase: "recheck",
                    safe_cause: "writing_project_compile_state_changed".into(),
                    sqlite_code: None,
                    sqlite_extended_code: None,
                    original_sql_failure: None,
                },
            )));
        }
        Ok(())
    }
    pub(super) fn delete(
        &self,
        tx: &Transaction<'_>,
        id: &str,
    ) -> Result<(), Box<PluginDataError>> {
        write_in(tx, &self.delete, &self.call(&self.delete, id)).map(|_| ())
    }
}
