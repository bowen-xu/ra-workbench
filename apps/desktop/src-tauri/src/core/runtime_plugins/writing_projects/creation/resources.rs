//! Fixed original creation statements, bound to one accepted writing package.
use crate::{
    data_interface::plugin_resources::{ColumnCodec, PluginDataError, PreparedDataRead, write_in},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataResourceGrant, DataValue},
    writing_project::{CreationPlan, Metadata, ProjectScope},
};
use rusqlite::Transaction;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;

pub(super) struct CreationResources {
    parent: AcceptedDataResource,
    standalone: AcceptedDataResource,
    insert: AcceptedDataResource,
    file: AcceptedDataResource,
    touch: AcceptedDataResource,
    delete: AcceptedDataResource,
}

impl CreationResources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let text = ColumnCodec::String.field();
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
        Ok(Self {
            parent: load(
                "parent-exists",
                "9d7fb0dfc0ce5b4d24176d3483315a697c03e8746a27538258ec72f17c086620",
                DataAccess::Read,
                vec![text],
                vec![ColumnCodec::Bool.field()],
            )?,
            standalone: load(
                "standalone-upsert",
                "67e4118df63c13d0655238058c997ff1cf9808da5547b670ceaee8c8f6c93464",
                DataAccess::Write,
                vec![text],
                vec![],
            )?,
            insert: load(
                "insert",
                "ea07a86cebbe2c9e27c9772056a9173f0508569aa21dca78e880fec694a8124b",
                DataAccess::Write,
                vec![text; 4],
                vec![],
            )?,
            file: load(
                "file-upsert",
                "e0af4ccfc7fc60921abaa5f3fe7380d25eff58e882e48953788f6475120058da",
                DataAccess::Write,
                vec![text, text, text, ColumnCodec::I64.field(), text],
                vec![],
            )?,
            touch: load(
                "touch",
                "96589127b930fbaef08504cf202a7f5106694453e23c28e5df2c19239ad37989",
                DataAccess::Write,
                vec![text],
                vec![],
            )?,
            delete: load(
                "delete",
                "700b4e488d89df911fb0ea5851045f3e3e241e8905424c855135642139ffc1b3",
                DataAccess::Write,
                vec![text],
                vec![],
            )?,
        })
    }

    pub(super) fn capture_parent(
        &self,
        tx: &Transaction<'_>,
        project: &str,
    ) -> Result<PreparedDataRead> {
        PreparedDataRead::capture(
            tx,
            self.parent.clone(),
            self.call(&self.parent, vec![DataValue::Text(project.into())]),
        )
    }

    fn call(&self, resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments,
        }
    }

    pub(super) fn standalone(&self, tx: &Transaction<'_>, project: &str) -> Result<()> {
        write_in(
            tx,
            &self.standalone,
            &self.call(&self.standalone, vec![DataValue::Text(project.into())]),
        )
        .map(|_| ())
    }

    pub(super) fn insert(
        &self,
        tx: &Transaction<'_>,
        plan: &CreationPlan,
        metadata: &Metadata,
    ) -> Result<()> {
        let project = match &plan.scope {
            ProjectScope::Existing { project_id } | ProjectScope::Standalone { project_id } => {
                project_id
            }
        };
        write_in(
            tx,
            &self.insert,
            &self.call(
                &self.insert,
                [
                    &plan.manuscript_id,
                    project,
                    &metadata.title,
                    &metadata.root_path,
                ]
                .into_iter()
                .map(|value| DataValue::Text(value.clone()))
                .collect(),
            ),
        )
        .map(|_| ())
    }

    pub(super) fn index(
        &self,
        tx: &Transaction<'_>,
        id: &str,
        path: &str,
        kind: &str,
        size: usize,
        hash: &str,
    ) -> Result<()> {
        // Template size is bounded by the original title limit and format.
        let size = size as i64;
        write_in(
            tx,
            &self.file,
            &self.call(
                &self.file,
                vec![
                    DataValue::Text(id.into()),
                    DataValue::Text(path.into()),
                    DataValue::Text(kind.into()),
                    DataValue::Integer(size),
                    DataValue::Text(hash.into()),
                ],
            ),
        )?;
        write_in(
            tx,
            &self.touch,
            &self.call(&self.touch, vec![DataValue::Text(id.into())]),
        )
        .map(|_| ())
    }

    pub(super) fn delete(&self, tx: &Transaction<'_>, id: &str) -> Result<()> {
        write_in(
            tx,
            &self.delete,
            &self.call(&self.delete, vec![DataValue::Text(id.into())]),
        )
        .map(|_| ())
    }
}
