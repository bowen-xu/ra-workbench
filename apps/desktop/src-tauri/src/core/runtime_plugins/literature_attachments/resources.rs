//! Fixed attachment statements on the caller's original connection and transaction.
//! The accepted method never grants a raw path, SQL connection or new IO owner.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataRows, DataType, DataValue,
};
use rusqlite::Connection;
use std::collections::BTreeMap;

mod specs;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;

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
const REAL: DataField = DataField {
    data_type: DataType::Real,
    nullable: false,
};

struct Spec {
    key: &'static str,
    file: &'static str,
    id: &'static str,
    sha256: &'static str,
    access: DataAccess,
    parameters: &'static [DataField],
    columns: &'static [DataField],
    policy: RowPolicy,
    codecs: &'static [ColumnCodec],
}

#[derive(Clone)]
pub(crate) struct AttachmentResources {
    package_key: String,
    statements: BTreeMap<&'static str, AcceptedDataResource>,
}

impl AttachmentResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for spec in specs::SPECS {
            let resource = load_statement(catalog, package, spec)?;
            statements.insert(spec.key, resource);
        }
        Ok(Self {
            package_key: package.package_key.clone(),
            statements,
        })
    }

    /// Browser capture reuses the original attachment write in its own Deferred transaction.
    pub(crate) fn load_attach(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for spec in specs::SPECS.iter().filter(|spec| {
            matches!(
                spec.key,
                "active-parent" | "same-item-hash" | "insert" | "other-items" | "duplicate-insert"
            )
        }) {
            statements.insert(spec.key, load_statement(catalog, package, spec)?);
        }
        Ok(Self {
            package_key: package.package_key.clone(),
            statements,
        })
    }

    /// Docling consumes only this already-published read resource.
    pub(crate) fn load_content_path(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let spec = &specs::CONTENT_PATH;
        let resource = load_statement(catalog, package, spec)?;
        Ok(Self {
            package_key: package.package_key.clone(),
            statements: BTreeMap::from([(spec.key, resource)]),
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

    /// Row policies and original row.get codecs are fixed Core metadata. They
    /// cannot be selected by a Native request, nor do they start a transaction.
    pub(crate) fn read(
        &self,
        db: &Connection,
        key: &str,
        arguments: Vec<DataValue>,
    ) -> Result<DataRows> {
        let spec = specs::SPECS
            .iter()
            .find(|spec| spec.key == key)
            .ok_or_else(|| self.shape_error(key))?;
        let resource = self.resource(key)?;
        read_on_connection(
            db,
            resource,
            &call(resource, arguments),
            spec.policy,
            spec.codecs,
        )
    }

    /// Keep ordinary attach/rename/position writes in autocommit and remove/
    /// index writes in their caller's existing transaction. Counts and failures
    /// do not assert that a trigger had no side effects.
    pub(crate) fn write(
        &self,
        db: &Connection,
        key: &str,
        arguments: Vec<DataValue>,
    ) -> Result<u64> {
        let resource = self.resource(key)?;
        write_on_connection(db, resource, &call(resource, arguments))
    }

    pub(crate) fn shape_error(&self, key: &str) -> Box<PluginDataError> {
        let id = self.statements.get(key).map_or_else(
            || format!("literature.attachments.{key}"),
            |resource| resource.grant().id.clone(),
        );
        Box::new(PluginDataError {
            code: "plugin_data_result_incompatible",
            message: "附件数据与固定契约不符。",
            package_key: self.package_key.clone(),
            resource_id: id,
            resource_version: 1,
            phase: "read",
            safe_cause: "attachment_data_shape".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        })
    }
}

fn load_statement(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    spec: &Spec,
) -> std::result::Result<AcceptedDataResource, RuntimePluginError> {
    AcceptedDataResource::load(
        catalog,
        package,
        "literature_library",
        &format!("data/{}", spec.file),
        DataResourceGrant {
            id: spec.id.into(),
            version: 1,
            sha256: spec.sha256.into(),
            access: spec.access,
            parameters: spec.parameters.to_vec(),
            columns: spec.columns.to_vec(),
        },
    )
}

fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}

#[cfg(test)]
mod tests;
