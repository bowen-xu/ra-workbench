//! Original recovery statements, in their original autocommit order.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataResourceGrant, DataRows, DataValue},
    writing_project::RecoveryDraftInput,
};
use rusqlite::Connection;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;
const COLUMNS: [ColumnCodec; 5] = [ColumnCodec::String; 5];

pub(super) struct RecoveryResources {
    list: AcceptedDataResource,
    get: AcceptedDataResource,
    upsert: AcceptedDataResource,
    expire: AcceptedDataResource,
    prune_manuscript: AcceptedDataResource,
    prune_global: AcceptedDataResource,
    clear: AcceptedDataResource,
}

impl RecoveryResources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let text = ColumnCodec::String.field();
        let integer = ColumnCodec::I64.field();
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
        let columns = || COLUMNS.iter().map(|codec| codec.field()).collect();
        Ok(Self {
            list: load(
                "drafts",
                "dea7c7c1dcd63a13cd5c078978bb42f0747c90e373e2438dc2aac09c1e81bed6",
                DataAccess::Read,
                vec![text],
                columns(),
            )?,
            get: load(
                "draft-get",
                "e157e6a85c0ce5ff73edc52157af696b304825d2162e89739e556de29b28e803",
                DataAccess::Read,
                vec![text; 2],
                columns(),
            )?,
            upsert: load(
                "draft-upsert",
                "771b34a52f44f195b111aaf004830a5a766c212019c47f0c8dc43873a88e5c17",
                DataAccess::Write,
                vec![text; 4],
                vec![],
            )?,
            expire: load(
                "draft-expire",
                "d4ae09d33c0a6cc994edecbf6add740efb233f4549dac1b3200b371b868c5e6c",
                DataAccess::Write,
                vec![],
                vec![],
            )?,
            prune_manuscript: load(
                "draft-prune-manuscript",
                "65aa345781f59ee18fa782d5ea9b7cbaf69f1e57d3c922ce72cab37a56a0380f",
                DataAccess::Write,
                vec![text, integer],
                vec![],
            )?,
            prune_global: load(
                "draft-prune-global",
                "a2eebbce3362a7c399b69eb0e9a3ac7c5f0a6279e51aeaa09a59975ea7d6ff02",
                DataAccess::Write,
                vec![integer],
                vec![],
            )?,
            clear: load(
                "draft-delete",
                "6d48fb5e85dde34efb4ebf6fd2ecad7a9f69616992f26ba0965b03e62ac07acb",
                DataAccess::Write,
                vec![text; 2],
                vec![],
            )?,
        })
    }

    fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments,
        }
    }

    pub(super) fn list(&self, db: &Connection, id: &str) -> Result<DataRows> {
        read_on_connection(
            db,
            &self.list,
            &Self::call(&self.list, vec![DataValue::Text(id.into())]),
            RowPolicy::All,
            &COLUMNS,
        )
    }

    pub(super) fn save(
        &self,
        db: &Connection,
        draft: &RecoveryDraftInput,
        per_manuscript: i64,
        total: i64,
    ) -> Result<DataRows> {
        write_on_connection(
            db,
            &self.upsert,
            &Self::call(
                &self.upsert,
                [
                    &draft.manuscript_id,
                    &draft.relative_path,
                    &draft.base_sha256,
                    &draft.content,
                ]
                .into_iter()
                .map(|value| DataValue::Text(value.clone()))
                .collect(),
            ),
        )?;
        write_on_connection(db, &self.expire, &Self::call(&self.expire, vec![]))?;
        write_on_connection(
            db,
            &self.prune_manuscript,
            &Self::call(
                &self.prune_manuscript,
                vec![
                    DataValue::Text(draft.manuscript_id.clone()),
                    DataValue::Integer(per_manuscript),
                ],
            ),
        )?;
        write_on_connection(
            db,
            &self.prune_global,
            &Self::call(&self.prune_global, vec![DataValue::Integer(total)]),
        )?;
        read_on_connection(
            db,
            &self.get,
            &Self::call(
                &self.get,
                vec![
                    DataValue::Text(draft.manuscript_id.clone()),
                    DataValue::Text(draft.relative_path.clone()),
                ],
            ),
            RowPolicy::FirstRequired,
            &COLUMNS,
        )
    }

    pub(super) fn clear(&self, db: &Connection, id: &str, path: &str) -> Result<()> {
        write_on_connection(
            db,
            &self.clear,
            &Self::call(
                &self.clear,
                vec![DataValue::Text(id.into()), DataValue::Text(path.into())],
            ),
        )
        .map(|_| ())
    }
}
