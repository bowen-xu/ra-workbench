//! Exact plugin-owned SQL resources; no runtime SQL selection is accepted.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_in,
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
struct Spec {
    key: &'static str,
    file: &'static str,
    id: &'static str,
    sha: &'static str,
    access: DataAccess,
    args: &'static [DataField],
    cols: &'static [DataField],
}
#[derive(Clone)]
pub(crate) struct ConceptResources {
    package_key: String,
    resources: BTreeMap<&'static str, AcceptedDataResource>,
}
impl ConceptResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut resources = BTreeMap::new();
        for s in specs::SPECS {
            resources.insert(
                s.key,
                AcceptedDataResource::load(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{}", s.file),
                    DataResourceGrant {
                        id: s.id.into(),
                        version: 1,
                        sha256: s.sha.into(),
                        access: s.access,
                        parameters: s.args.to_vec(),
                        columns: s.cols.to_vec(),
                    },
                )?,
            );
        }
        Ok(Self {
            package_key: package.package_key.clone(),
            resources,
        })
    }
    pub(crate) fn package_key(&self) -> &str {
        &self.package_key
    }
    pub(super) fn resource(&self, key: &str) -> &AcceptedDataResource {
        self.resources
            .get(key)
            .expect("only fixed concept resource keys are used by the typed adapter")
    }
    pub(super) fn read(
        &self,
        db: &Connection,
        key: &str,
        args: Vec<DataValue>,
        policy: RowPolicy,
        codecs: &[ColumnCodec],
    ) -> Result<DataRows, Box<PluginDataError>> {
        let r = self.resource(key);
        read_on_connection(db, r, &call(r, args), policy, codecs)
    }
    pub(super) fn write(
        &self,
        tx: &Transaction<'_>,
        key: &str,
        args: Vec<DataValue>,
    ) -> Result<u64, Box<PluginDataError>> {
        let r = self.resource(key);
        write_in(tx, r, &call(r, args))
    }
}
pub(super) fn call(r: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: r.grant().id.clone(),
        version: r.grant().version,
        arguments,
    }
}
