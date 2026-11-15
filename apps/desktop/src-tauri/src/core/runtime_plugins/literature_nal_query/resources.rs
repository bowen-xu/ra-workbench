use super::QueryCause;
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_in,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue},
    nal_query::{SavePlan, StoredRow},
};
use rusqlite::{Connection, Transaction};

const PREFIX: &str = "literature.nal-query-collections";
const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const OPTIONAL_TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};
const INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};

#[derive(Clone)]
pub(crate) struct NalQueryResources {
    read: AcceptedDataResource,
    write: AcceptedDataResource,
}

impl NalQueryResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, hash: &str, access, parameters, columns| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/nal-query-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("{PREFIX}.{name}"),
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
                "4664f7969654a4c47fe5dc166bfb25b3d53179687a80c048cf27e44cbb92c49e",
                DataAccess::Read,
                vec![],
                vec![INTEGER, TEXT, OPTIONAL_TEXT, INTEGER],
            )?,
            write: load(
                "save",
                "43bf8baa668e43bcad010a92acf41c4c5d0723830a5fe89d905abd70bb90b110",
                DataAccess::Write,
                vec![TEXT, OPTIONAL_TEXT, INTEGER, INTEGER],
                vec![],
            )?,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        self.read.package_key()
    }

    /// The original query_row's first row and primitive conversions, with no BEGIN
    /// or JSON/semantic interpretation. An absent singleton is never an empty store.
    pub(crate) fn read_raw(&self, connection: &Connection) -> Result<StoredRow, QueryCause> {
        let call = call(&self.read, vec![]);
        let rows = read_on_connection(
            connection,
            &self.read,
            &call,
            RowPolicy::FirstRequired,
            &[
                ColumnCodec::I64,
                ColumnCodec::String,
                ColumnCodec::OptionalString,
                ColumnCodec::Bool,
            ],
        )
        .map_err(QueryCause::Data)?;
        let invalid = || {
            QueryCause::Data(Box::new(PluginDataError {
                code: "plugin_data_result_incompatible",
                message: "查询集合结果不符合固定列类型。",
                package_key: self.package_key().into(),
                resource_id: self.read.grant().id.clone(),
                resource_version: 1,
                phase: "decode",
                safe_cause: "nal_query_row_shape".into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            }))
        };
        let row: [DataValue; 4] = rows
            .rows
            .into_iter()
            .next()
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?;
        let [
            DataValue::Integer(revision),
            DataValue::Text(definitions_json),
            cache,
            DataValue::Integer(legacy),
        ] = row
        else {
            return Err(invalid());
        };
        let cache_json = match cache {
            DataValue::Null => None,
            DataValue::Text(value) => Some(value),
            _ => return Err(invalid()),
        };
        Ok(StoredRow {
            revision,
            definitions_json,
            cache_json,
            legacy_imported: legacy != 0,
        })
    }

    pub(super) fn update_in(
        &self,
        tx: &Transaction<'_>,
        plan: &SavePlan,
        revision: i64,
    ) -> Result<u64, QueryCause> {
        let request = call(
            &self.write,
            vec![
                DataValue::Text(plan.definitions_json.clone()),
                plan.cache_json
                    .as_ref()
                    .map_or(DataValue::Null, |value| DataValue::Text(value.clone())),
                DataValue::Integer(i64::from(plan.legacy_imported)),
                DataValue::Integer(revision),
            ],
        );
        write_in(tx, &self.write, &request).map_err(QueryCause::Data)
    }
}

fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}
