//! Fixed metadata statements; callers retain the original transaction and order.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_in as execute_write_in,
        write_on_connection as execute_write_on_connection,
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

mod metadata;
mod reused;
mod scalar;

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
const OPTIONAL_INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: true,
};
const REAL: DataField = DataField {
    data_type: DataType::Real,
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
pub(crate) struct MetadataResources {
    package_key: String,
    statements: BTreeMap<&'static str, AcceptedDataResource>,
}

impl MetadataResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        load_fixed(
            catalog,
            package,
            "metadata",
            "literature.metadata",
            scalar::SPECS,
            &mut statements,
        )?;
        load_fixed(
            catalog,
            package,
            "metadata",
            "literature.metadata",
            metadata::SPECS,
            &mut statements,
        )?;
        // Reuse the original byte-identical grants, without CatalogSave provenance.
        load_fixed(
            catalog,
            package,
            "catalog-save",
            "literature.catalog-save",
            reused::SPECS,
            &mut statements,
        )?;
        Ok(Self {
            package_key: package.package_key.clone(),
            statements,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        &self.package_key
    }

    /// Keys are fixed Core call sites, never resource names selected by a child.
    pub(crate) fn resource(&self, logical_key: &str) -> Result<&AcceptedDataResource> {
        self.statements.get(logical_key).ok_or_else(|| {
            Box::new(PluginDataError {
                code: "plugin_data_resource_unavailable",
                message: "元数据固定资源不可用。",
                package_key: self.package_key.clone(),
                resource_id: format!("literature.metadata.{logical_key}"),
                resource_version: 1,
                phase: "resource",
                safe_cause: "metadata_resource_not_granted".into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            })
        })
    }

    /// Row selection and codecs are chosen by the original typed Core caller.
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

    pub(crate) fn write_in(
        &self,
        tx: &Transaction<'_>,
        key: &str,
        arguments: Vec<DataValue>,
    ) -> Result<u64> {
        let resource = self.resource(key)?;
        execute_write_in(tx, resource, &call(resource, arguments))
    }

    pub(crate) fn write_on_connection(
        &self,
        db: &Connection,
        key: &str,
        arguments: Vec<DataValue>,
    ) -> Result<u64> {
        let resource = self.resource(key)?;
        execute_write_on_connection(db, resource, &call(resource, arguments))
    }
}

fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}

fn load_fixed(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    path_prefix: &str,
    id_prefix: &str,
    specs: &[Spec],
    statements: &mut BTreeMap<&'static str, AcceptedDataResource>,
) -> std::result::Result<(), RuntimePluginError> {
    for spec in specs {
        let resource = AcceptedDataResource::load(
            catalog,
            package,
            "literature_library",
            &format!("data/{path_prefix}-{}.v1.sql", spec.key),
            DataResourceGrant {
                id: format!("{id_prefix}.{}", spec.key),
                version: 1,
                sha256: spec.sha256.into(),
                access: spec.access,
                parameters: spec.parameters.to_vec(),
                columns: spec.columns.to_vec(),
            },
        )?;
        statements.insert(spec.key, resource);
    }
    Ok(())
}
