use crate::{
    data_interface::plugin_resources::{self, ColumnCodec, PluginDataError, RowPolicy},
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
pub(crate) struct PdfImportResources {
    package_key: String,
    statements: BTreeMap<&'static str, AcceptedDataResource>,
}

impl PdfImportResources {
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
                &format!("data/pdf-import-{}.v1.sql", spec.key),
                DataResourceGrant {
                    id: format!("literature.pdf-import.{}", spec.key),
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
        self.statements.get(key).ok_or_else(|| {
            Box::new(PluginDataError {
                code: "plugin_data_resource_unavailable",
                message: "PDF导入固定资源不可用。",
                package_key: self.package_key.clone(),
                resource_id: format!("literature.pdf-import.{key}"),
                resource_version: 1,
                phase: "resource",
                safe_cause: "pdf_import_resource_not_granted".into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            })
        })
    }
    pub(crate) fn read(
        &self,
        db: &Connection,
        key: &str,
        arguments: Vec<DataValue>,
        policy: RowPolicy,
        codecs: &[ColumnCodec],
    ) -> Result<DataRows> {
        let resource = self.resource(key)?;
        plugin_resources::read_on_connection(
            db,
            resource,
            &call(resource, arguments),
            policy,
            codecs,
        )
    }
    pub(crate) fn write(
        &self,
        db: &Connection,
        key: &str,
        arguments: Vec<DataValue>,
    ) -> Result<u64> {
        let resource = self.resource(key)?;
        plugin_resources::write_on_connection(db, resource, &call(resource, arguments))
    }
    pub(crate) fn write_in(
        &self,
        tx: &Transaction<'_>,
        key: &str,
        arguments: Vec<DataValue>,
    ) -> Result<u64> {
        let resource = self.resource(key)?;
        plugin_resources::write_in(tx, resource, &call(resource, arguments))
    }
}
fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}
