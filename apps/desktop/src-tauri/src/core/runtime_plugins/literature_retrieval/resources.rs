//! Fixed retrieval statements. No database, statement selector or SQL is public.
use crate::{
    data_interface::plugin_resources::{self, ColumnCodec, PluginDataError, RowPolicy},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataRows, DataType, DataValue,
};
use rusqlite::{Connection, Params};
use std::collections::BTreeMap;
mod specs;
const T: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const N: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};
const I: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};
const NI: DataField = DataField {
    data_type: DataType::Integer,
    nullable: true,
};
const R: DataField = DataField {
    data_type: DataType::Real,
    nullable: false,
};
const NR: DataField = DataField {
    data_type: DataType::Real,
    nullable: true,
};
struct Spec {
    key: &'static str,
    file: &'static str,
    sha256: &'static str,
    access: DataAccess,
    parameters: &'static [DataField],
    codecs: &'static [ColumnCodec],
    policy: RowPolicy,
}
#[derive(Clone)]
pub(crate) struct RetrievalResources {
    resources: BTreeMap<&'static str, AcceptedDataResource>,
}
struct Observed {
    key: &'static str,
    arguments: Vec<DataValue>,
    rows: DataRows,
}
#[derive(Default)]
pub(super) struct ReadSet(Vec<Observed>);
impl RetrievalResources {
    pub fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut resources = BTreeMap::new();
        for spec in specs::SPECS {
            resources.insert(
                spec.key,
                AcceptedDataResource::load(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{}", spec.file),
                    DataResourceGrant {
                        id: format!("literature.concept-retrieval.{}", spec.key),
                        version: 1,
                        sha256: spec.sha256.into(),
                        access: spec.access,
                        parameters: spec.parameters.to_vec(),
                        columns: spec.codecs.iter().map(|codec| codec.field()).collect(),
                    },
                )?,
            );
        }
        Ok(Self { resources })
    }
    fn resource(&self, key: &'static str) -> &AcceptedDataResource {
        self.resources
            .get(key)
            .expect("all fixed retrieval resources loaded together")
    }
    pub(super) fn read(
        &self,
        db: &Connection,
        key: &'static str,
        arguments: Vec<DataValue>,
    ) -> Result<DataRows, Box<PluginDataError>> {
        let spec = specs::SPECS
            .iter()
            .find(|spec| spec.key == key)
            .expect("Core selects a fixed retrieval statement");
        let resource = self.resource(key);
        let call = DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments,
        };
        if matches!(key, "definition-sources" | "usage-sources") {
            // The original row closures read their two context fields first.
            plugin_resources::read_on_connection_in_order(
                db,
                resource,
                &call,
                spec.policy,
                spec.codecs,
                &[10, 11, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            )
        } else {
            plugin_resources::read_on_connection(db, resource, &call, spec.policy, spec.codecs)
        }
    }
    pub(super) fn capture(
        &self,
        db: &Connection,
        reads: &mut ReadSet,
        key: &'static str,
        arguments: Vec<DataValue>,
    ) -> Result<DataRows, Box<PluginDataError>> {
        let rows = self.read(db, key, arguments.clone())?;
        reads.0.push(Observed {
            key,
            arguments,
            rows: rows.clone(),
        });
        Ok(rows)
    }
    pub(super) fn recheck(
        &self,
        db: &Connection,
        reads: &ReadSet,
    ) -> Result<(), Box<PluginDataError>> {
        for old in &reads.0 {
            if self.read(db, old.key, old.arguments.clone())? != old.rows {
                let resource = self.resource(old.key);
                return Err(Box::new(PluginDataError {
                    code: "plugin_data_snapshot_changed",
                    message: "检索读取来源已变化，请重新预览。",
                    package_key: resource.package_key().into(),
                    resource_id: resource.grant().id.clone(),
                    resource_version: 1,
                    phase: "recheck",
                    safe_cause: "complete_read_set_changed".into(),
                    sqlite_code: None,
                    sqlite_extended_code: None,
                    original_sql_failure: None,
                }));
            }
        }
        Ok(())
    }
    pub(super) fn shape_error(&self, key: &'static str) -> Box<PluginDataError> {
        let resource = self.resource(key);
        Box::new(PluginDataError {
            code: "plugin_data_result_invalid",
            message: "检索数据格式与原声明不匹配。",
            package_key: resource.package_key().into(),
            resource_id: resource.grant().id.clone(),
            resource_version: 1,
            phase: "decode",
            safe_cause: "fixed_row_shape_mismatch".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        })
    }
    pub(super) fn execute(
        &self,
        db: &Connection,
        key: &'static str,
        values: impl Params,
    ) -> Result<usize, Box<PluginDataError>> {
        let resource = self.resource(key);
        // Typed original bindings preserve SQLite's f32/f64 and NULL behavior.
        // Native cannot choose these statements or provide arbitrary bindings.
        db.execute(resource.sql(), values)
            .map_err(|error| plugin_resources::sql_failure(resource, "write", &error))
    }
    pub(super) fn map_rows<T, E>(
        &self,
        db: &Connection,
        key: &'static str,
        arguments: Vec<DataValue>,
        map: impl FnMut(Vec<DataValue>) -> std::result::Result<T, E>,
    ) -> std::result::Result<Vec<T>, plugin_resources::MappedReadError<E>> {
        let spec = specs::SPECS
            .iter()
            .find(|spec| spec.key == key)
            .expect("Core selects a fixed retrieval statement");
        let resource = self.resource(key);
        plugin_resources::read_on_connection_mapped(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments,
            },
            spec.policy,
            spec.codecs,
            map,
        )
    }
}
