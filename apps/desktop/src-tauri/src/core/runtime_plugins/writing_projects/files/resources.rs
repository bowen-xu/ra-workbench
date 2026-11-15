//! Original file-list and citation-redo statements with fixed Core grants.
use super::super::resources::ProjectReadResources;
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
use rusqlite::Connection;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;
const FILE_COLUMNS: [ColumnCodec; 6] = [
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::I64,
    ColumnCodec::String,
    ColumnCodec::String,
];

pub(crate) fn load_file_list(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
) -> std::result::Result<AcceptedDataResource, RuntimePluginError> {
    AcceptedDataResource::load(
        catalog,
        package,
        "writing_projects",
        "data/project-files.v1.sql",
        DataResourceGrant {
            id: "writing.projects.files".into(),
            version: 1,
            sha256: "deebbd3b40405b9de6a1a5b44ec49903b0edffdbd9a173625497bd12fc539095".into(),
            access: DataAccess::Read,
            parameters: vec![ColumnCodec::String.field()],
            columns: FILE_COLUMNS.iter().map(|codec| codec.field()).collect(),
        },
    )
}

pub(crate) fn read_file_list(
    db: &Connection,
    resource: &AcceptedDataResource,
    id: &str,
) -> Result<DataRows> {
    read_on_connection(
        db,
        resource,
        &DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: vec![DataValue::Text(id.into())],
        },
        RowPolicy::All,
        &FILE_COLUMNS,
    )
}

pub(crate) struct FileResources {
    pub(crate) projects: ProjectReadResources,
    pub(super) files: AcceptedDataResource,
    pending: AcceptedDataResource,
    clear: AcceptedDataResource,
    index: AcceptedDataResource,
    touch: AcceptedDataResource,
}

impl FileResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
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
            projects: ProjectReadResources::load(catalog, package)?,
            files: load_file_list(catalog, package)?,
            pending: load(
                "pending-citation-writes",
                "769b2dcdac8499f2828f31764d64863e0c8154f71db72cedd3c89d899d34bdba",
                DataAccess::Read,
                vec![text],
                vec![text],
            )?,
            clear: load(
                "clear-citation-writes",
                "6524e8dd4c35b36242d03109c4d48b006a37ce776e35aa127846705c1c4d526d",
                DataAccess::Write,
                vec![text],
                vec![],
            )?,
            index: load(
                "file-upsert",
                "e0af4ccfc7fc60921abaa5f3fe7380d25eff58e882e48953788f6475120058da",
                DataAccess::Write,
                vec![text, text, text, ColumnCodec::I64.field(), text],
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

    fn call(resource: &AcceptedDataResource, id: &str) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: vec![DataValue::Text(id.into())],
        }
    }

    pub(super) fn pending(&self, db: &Connection, id: &str) -> Result<DataRows> {
        read_on_connection(
            db,
            &self.pending,
            &Self::call(&self.pending, id),
            RowPolicy::FirstOptional,
            &[ColumnCodec::String],
        )
    }

    pub(super) fn recheck_pending(
        &self,
        db: &Connection,
        id: &str,
        captured: &DataRows,
    ) -> Result<()> {
        if self.pending(db, id)? != *captured {
            return Err(Box::new(PluginDataError {
                code: "plugin_data_snapshot_changed",
                message: "插件计算所依据的数据已变化；本次尚未提交。",
                package_key: self.pending.package_key().into(),
                resource_id: self.pending.grant().id.clone(),
                resource_version: 1,
                phase: "recheck",
                safe_cause: "writing_citation_redo_read_set_changed".into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            }));
        }
        Ok(())
    }

    pub(crate) fn files(&self, db: &Connection, id: &str) -> Result<DataRows> {
        read_file_list(db, &self.files, id)
    }

    pub(crate) fn index(
        &self,
        db: &Connection,
        id: &str,
        path: &str,
        kind: &str,
        size: usize,
        hash: &str,
    ) -> Result<()> {
        write_on_connection(
            db,
            &self.index,
            &DataCall {
                id: self.index.grant().id.clone(),
                version: 1,
                arguments: vec![
                    DataValue::Text(id.into()),
                    DataValue::Text(path.into()),
                    DataValue::Text(kind.into()),
                    DataValue::Integer(size as i64),
                    DataValue::Text(hash.into()),
                ],
            },
        )?;
        self.touch(db, id)
    }

    pub(crate) fn touch(&self, db: &Connection, id: &str) -> Result<()> {
        write_on_connection(db, &self.touch, &Self::call(&self.touch, id)).map(|_| ())
    }

    pub(super) fn clear(&self, db: &Connection, id: &str) -> Result<()> {
        write_on_connection(db, &self.clear, &Self::call(&self.clear, id)).map(|_| ())
    }
}
