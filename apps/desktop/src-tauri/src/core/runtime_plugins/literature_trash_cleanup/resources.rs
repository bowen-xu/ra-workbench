//! Core metadata grants, never SQL text or a child-selected statement.
use super::TrashCleanupProblem;
use crate::{
    data_interface::plugin_resources::{ColumnCodec, RowPolicy, read_on_connection, write_in},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataRows, DataType, DataValue,
};
use rusqlite::{Connection, Transaction};
use std::collections::BTreeMap;

mod fixed;
mod outer;

const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};
struct WriteSpec {
    name: &'static str,
    sha256: &'static str,
    arguments: usize,
    post: bool,
    parent_index: Option<usize>,
}

#[derive(Clone)]
pub(crate) struct TrashCleanupResources {
    statements: BTreeMap<String, AcceptedDataResource>,
}

impl TrashCleanupResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for spec in fixed::specs() {
            let resource = load(
                catalog,
                package,
                spec.name,
                spec.sha256,
                DataAccess::Write,
                vec![TEXT; spec.arguments],
                vec![],
            )?;
            statements.insert(resource.grant().id.clone(), resource);
        }
        outer::load(catalog, package, &mut statements)?;
        Ok(Self { statements })
    }

    pub(crate) fn package_key(&self) -> &str {
        // Successful construction always contains the fixed root grant.
        self.statements
            .values()
            .next()
            .expect("fixed resources are nonempty")
            .package_key()
    }

    pub(super) fn resource(
        &self,
        name: &str,
    ) -> Result<&AcceptedDataResource, TrashCleanupProblem> {
        self.resource_id(&format!("literature.trash-cleanup.{name}"))
    }

    pub(super) fn resource_id(
        &self,
        id: &str,
    ) -> Result<&AcceptedDataResource, TrashCleanupProblem> {
        self.statements.get(id).ok_or(TrashCleanupProblem::Binding)
    }

    pub(super) fn read(
        &self,
        db: &Connection,
        name: &str,
        arguments: Vec<DataValue>,
        policy: RowPolicy,
        codecs: &[ColumnCodec],
    ) -> Result<DataRows, TrashCleanupProblem> {
        let resource = self.resource(name)?;
        read_on_connection(db, resource, &call(resource, arguments), policy, codecs)
            .map_err(TrashCleanupProblem::Data)
    }

    pub(super) fn write(
        &self,
        tx: &Transaction<'_>,
        name: &str,
        arguments: Vec<DataValue>,
    ) -> Result<u64, TrashCleanupProblem> {
        let resource = self.resource(name)?;
        write_in(tx, resource, &call(resource, arguments)).map_err(TrashCleanupProblem::Data)
    }

    pub(super) fn post_specs(&self) -> impl Iterator<Item = (&'static str, Option<usize>)> {
        fixed::specs()
            .filter(|spec| spec.post)
            .map(|spec| (spec.name, spec.parent_index))
    }
}

pub(super) fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}

fn load(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    name: &str,
    sha256: &str,
    access: DataAccess,
    parameters: Vec<DataField>,
    columns: Vec<DataField>,
) -> Result<AcceptedDataResource, RuntimePluginError> {
    AcceptedDataResource::load(
        catalog,
        package,
        "literature_library",
        &format!("data/trash-cleanup-{name}.v1.sql"),
        DataResourceGrant {
            id: format!("literature.trash-cleanup.{name}"),
            version: 1,
            sha256: sha256.into(),
            access,
            parameters,
            columns,
        },
    )
}
