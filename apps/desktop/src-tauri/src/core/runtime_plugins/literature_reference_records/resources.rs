//! Original reference-record statements, accepted from one selected package.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, read_on_connection_in_order,
        write_in,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataRows, DataType, DataValue,
};
use rusqlite::{Connection, Transaction};
use std::collections::BTreeMap;

mod specs;

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

struct Spec {
    key: &'static str,
    sha256: &'static str,
    access: DataAccess,
    parameters: &'static [DataField],
    columns: &'static [DataField],
}

type Result<T> = std::result::Result<T, Box<PluginDataError>>;

#[derive(Clone)]
pub(crate) struct ReferenceResources {
    package_key: String,
    statements: BTreeMap<&'static str, AcceptedDataResource>,
}

impl ReferenceResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for spec in specs::SPECS {
            let resource = AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/reference-records-{}.v1.sql", spec.key),
                DataResourceGrant {
                    id: format!("literature.reference-records.{}", spec.key),
                    version: 1,
                    sha256: spec.sha256.into(),
                    access: spec.access,
                    parameters: spec.parameters.to_vec(),
                    columns: spec.columns.to_vec(),
                },
            )?;
            statements.insert(spec.key, resource);
        }
        Ok(Self {
            package_key: package.package_key.clone(),
            statements,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        &self.package_key
    }

    fn resource(&self, key: &str) -> Result<&AcceptedDataResource> {
        self.statements
            .get(key)
            .ok_or_else(|| self.shape_error(key))
    }

    pub(crate) fn shape_error(&self, key: &str) -> Box<PluginDataError> {
        Box::new(PluginDataError {
            code: "plugin_data_result_incompatible",
            message: "参考记录数据与固定契约不符。",
            package_key: self.package_key.clone(),
            resource_id: format!("literature.reference-records.{key}"),
            resource_version: 1,
            phase: "read",
            safe_cause: "reference_record_data_shape".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        })
    }

    /// Selection and row codecs belong to the fixed Core caller, not the child.
    pub(crate) fn read(
        &self,
        db: &Connection,
        key: &str,
        arguments: Vec<DataValue>,
        policy: RowPolicy,
        codecs: &[ColumnCodec],
    ) -> Result<DataRows> {
        let resource = self.resource(key)?;
        read_on_connection(db, resource, &call(resource, arguments), policy, codecs)
    }

    pub(crate) fn write(
        &self,
        transaction: &Transaction<'_>,
        key: &str,
        arguments: Vec<DataValue>,
    ) -> Result<u64> {
        let resource = self.resource(key)?;
        write_in(transaction, resource, &call(resource, arguments))
    }

    pub(crate) fn record(&self, db: &Connection, id: &str) -> Result<DataRows> {
        use ColumnCodec::{I64, OptionalString, String};
        let resource = self.resource("record")?;
        // The original query closure built the record before reading column 4.
        // Keep that error precedence without changing the original SQL bytes.
        read_on_connection_in_order(
            db,
            resource,
            &call(resource, vec![DataValue::Text(id.into())]),
            RowPolicy::FirstOptional,
            &[
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                OptionalString,
                String,
                OptionalString,
                String,
                OptionalString,
                String,
                I64,
                String,
                String,
            ],
            &[0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 4],
        )
    }
}

fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}
