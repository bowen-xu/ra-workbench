use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataField, DataResourceGrant, DataRows, DataType, DataValue},
    duplicate_review::DuplicateDecisionRequestV1,
};
use rusqlite::Connection;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;
const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const CANDIDATES: &[ColumnCodec] = &[
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::F64Bits,
];

#[derive(Clone)]
pub(crate) struct DuplicateResources {
    candidates: AcceptedDataResource,
    history: AcceptedDataResource,
    decision: AcceptedDataResource,
}

impl DuplicateResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let load = |name: &str, id: &str, hash: &str, access, parameters, columns| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.duplicate-review.{id}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            candidates: load(
                "duplicate-candidates-list",
                "candidates-list",
                "9c54803c87939193c921523392160319cba4df269585a0a370974020f6c604b7",
                DataAccess::Read,
                vec![],
                vec![TEXT; 7],
            )?,
            history: load(
                "duplicate-merge-history",
                "merge-history",
                "2ba4edfbf0ddb01ebd152ec818effa02304e3e5b8d490713fe6bea4cd56c0300",
                DataAccess::Read,
                vec![],
                vec![TEXT; 7],
            )?,
            decision: load(
                "duplicate-candidate-decide",
                "candidate-decide",
                "b14dfd0e27351620cc3eb70799842748f25188b6f169f41bd3f1cac63ed044d7",
                DataAccess::Write,
                vec![TEXT; 2],
                vec![],
            )?,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        self.candidates.package_key()
    }

    pub(crate) fn candidates(&self, db: &Connection) -> Result<DataRows> {
        read_on_connection(
            db,
            &self.candidates,
            &call(&self.candidates, vec![]),
            RowPolicy::All,
            CANDIDATES,
        )
    }

    pub(crate) fn history(&self, db: &Connection) -> Result<DataRows> {
        read_on_connection(
            db,
            &self.history,
            &call(&self.history, vec![]),
            RowPolicy::All,
            &[ColumnCodec::String; 7],
        )
    }

    pub(super) fn decide(
        &self,
        db: &Connection,
        request: &DuplicateDecisionRequestV1,
    ) -> Result<u64> {
        write_on_connection(
            db,
            &self.decision,
            &call(
                &self.decision,
                vec![
                    DataValue::Text(request.candidate_id.clone()),
                    DataValue::Text(request.decision.clone()),
                ],
            ),
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
