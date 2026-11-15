//! Core grants for original package-owned project reads. SQL stays in the package.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataResourceGrant, DataRows, DataValue,
};
use research_workbench_native_sdk::writing_project::WritingSessionDraft;
use rusqlite::Connection;

const COLUMNS: [ColumnCodec; 7] = [ColumnCodec::String; 7];
#[derive(Clone)]
pub(super) struct ProjectReadResources {
    list: AcceptedDataResource,
    pub(super) get: AcceptedDataResource,
}
impl ProjectReadResources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, hash: &str, parameters| {
            AcceptedDataResource::load(
                catalog,
                package,
                "writing_projects",
                &format!("data/project-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("writing.projects.{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access: DataAccess::Read,
                    parameters,
                    columns: COLUMNS.iter().map(|codec| codec.field()).collect(),
                },
            )
        };
        Ok(Self {
            list: load(
                "list",
                "984682e0f0a6dc090417eff79de388e3beba03ae27248b54b9c4c7778370554d",
                vec![],
            )?,
            get: load(
                "get",
                "1a45b8c3b3897d30007ac62a0c174abdcebb60e609d7dc6d58ac4a689c7c1973",
                vec![ColumnCodec::String.field()],
            )?,
        })
    }
    pub(super) fn list(&self, db: &Connection) -> Result<DataRows, Box<PluginDataError>> {
        let call = DataCall {
            id: self.list.grant().id.clone(),
            version: 1,
            arguments: vec![],
        };
        read_on_connection(db, &self.list, &call, RowPolicy::All, &COLUMNS)
    }
    pub(super) fn get(&self, db: &Connection, id: &str) -> Result<DataRows, Box<PluginDataError>> {
        let call = DataCall {
            id: self.get.grant().id.clone(),
            version: 1,
            arguments: vec![DataValue::Text(id.into())],
        };
        read_on_connection(db, &self.get, &call, RowPolicy::FirstOptional, &COLUMNS)
    }

    /// The original single-statement metadata writer holds this same Core
    /// connection owner through recheck, autocommit UPDATE and read-back.
    pub(super) fn recheck(
        &self,
        db: &Connection,
        id: &str,
        captured: &DataRows,
    ) -> Result<(), Box<PluginDataError>> {
        if self.get(db, id)? != *captured {
            return Err(Box::new(PluginDataError {
                code: "plugin_data_snapshot_changed",
                message: "插件计算所依据的数据已变化；本次尚未提交。",
                package_key: self.get.package_key().into(),
                resource_id: self.get.grant().id.clone(),
                resource_version: 1,
                phase: "recheck",
                safe_cause: "writing_project_read_set_changed".into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            }));
        }
        Ok(())
    }
}

const SESSION_COLUMNS: [ColumnCodec; 6] = [
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::OptionalString,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
];

pub(super) struct SessionResources {
    pub(super) read: AcceptedDataResource,
    write: AcceptedDataResource,
}

impl SessionResources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let read = AcceptedDataResource::load(
            catalog,
            package,
            "writing_projects",
            "data/project-session.v1.sql",
            DataResourceGrant {
                id: "writing.projects.session".into(),
                version: 1,
                sha256: "5a32fdf1701c3c3dc143d3933f6687b3fa8aeeadf1581384732107679308ac32".into(),
                access: DataAccess::Read,
                parameters: vec![ColumnCodec::String.field()],
                columns: SESSION_COLUMNS.iter().map(|codec| codec.field()).collect(),
            },
        )?;
        let write = AcceptedDataResource::load(
            catalog,
            package,
            "writing_projects",
            "data/project-session-upsert.v1.sql",
            DataResourceGrant {
                id: "writing.projects.session-upsert".into(),
                version: 1,
                sha256: "64e18f0a28e6614a1a42a23632c66b307819e79ebfacb03572a1747864349ac0".into(),
                access: DataAccess::Write,
                parameters: SESSION_COLUMNS[..5]
                    .iter()
                    .map(|codec| codec.field())
                    .collect(),
                columns: vec![],
            },
        )?;
        Ok(Self { read, write })
    }

    pub(super) fn read(&self, db: &Connection, id: &str) -> Result<DataRows, Box<PluginDataError>> {
        let call = DataCall {
            id: self.read.grant().id.clone(),
            version: 1,
            arguments: vec![DataValue::Text(id.into())],
        };
        read_on_connection(
            db,
            &self.read,
            &call,
            RowPolicy::FirstOptional,
            &SESSION_COLUMNS,
        )
    }

    pub(super) fn save(
        &self,
        db: &Connection,
        draft: &WritingSessionDraft,
    ) -> crate::runtime_plugins::native::NativeResult<()> {
        let encoding_error = |error: serde_json::Error| {
            super::failure("storage_error", error.to_string(), self.write.package_key())
        };
        let open_paths = serde_json::to_string(&draft.open_paths).map_err(encoding_error)?;
        let cursor_state = serde_json::to_string(&draft.cursor_state).map_err(encoding_error)?;
        let layout_state = serde_json::to_string(&draft.layout_state).map_err(encoding_error)?;
        let call = DataCall {
            id: self.write.grant().id.clone(),
            version: 1,
            arguments: vec![
                DataValue::Text(draft.manuscript_id.clone()),
                DataValue::Text(open_paths),
                draft
                    .active_path
                    .clone()
                    .map_or(DataValue::Null, DataValue::Text),
                DataValue::Text(cursor_state),
                DataValue::Text(layout_state),
            ],
        };
        write_on_connection(db, &self.write, &call)
            .map(|_| ())
            .map_err(super::data_failure)
    }
}
