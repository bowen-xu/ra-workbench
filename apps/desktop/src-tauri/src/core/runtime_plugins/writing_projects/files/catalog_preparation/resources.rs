//! Original preparation SQL, with exact package bytes and row codecs.
use super::super::catalog::CatalogResources;
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataResourceGrant, DataRows, DataValue,
};
use rusqlite::Connection;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;
pub(super) struct Resources {
    pub(super) catalog: CatalogResources,
    retry: AcceptedDataResource,
    pending: AcceptedDataResource,
    current: AcceptedDataResource,
    counts: AcceptedDataResource,
    external: AcceptedDataResource,
    error: AcceptedDataResource,
}
impl Resources {
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
                &format!("data/project-note-prepare-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("writing.projects.note-prepare-{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            catalog: CatalogResources::load(catalog, package)?,
            retry: load(
                "retry",
                "9a2d45cf4bf02548192e0d1ee68b89c473f20d7c8fa1fd8ca71ab4aa9f344c75",
                DataAccess::Write,
                vec![text; 2],
                vec![],
            )?,
            pending: load(
                "pending",
                "8f03117178c930c98e37728d404c04209f991d31c6690cb8292096afab710687",
                DataAccess::Read,
                vec![text; 2],
                vec![text; 3],
            )?,
            current: load(
                "current",
                "db90cd6040af3a9bd886ac6b250d59ea6be38a59809db737ae6e735412ee0a29",
                DataAccess::Read,
                vec![text; 2],
                vec![text],
            )?,
            counts: load(
                "counts",
                "c50012082e928506ae367e988bed1c4459aa75d964a69435d19230b8cbedf534",
                DataAccess::Read,
                vec![text; 2],
                vec![ColumnCodec::I64.field(); 2],
            )?,
            external: load(
                "external",
                "d51a858c59106f8f5c0ae399471e4f2057e87d0f28e808e59464d9055be11f4f",
                DataAccess::Write,
                vec![text; 2],
                vec![],
            )?,
            error: load(
                "error",
                "e063ddd94c4f52d688987f5f9d3390a49a71c080164a50dbed34a42dec03244a",
                DataAccess::Write,
                vec![text; 3],
                vec![],
            )?,
        })
    }
    fn call(resource: &AcceptedDataResource, values: &[&str]) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: values
                .iter()
                .map(|value| DataValue::Text((*value).into()))
                .collect(),
        }
    }
    pub(super) fn retry(&self, db: &Connection, statuses: &[String; 2]) -> Result<()> {
        write_on_connection(
            db,
            &self.retry,
            &Self::call(&self.retry, &[&statuses[0], &statuses[1]]),
        )
        .map(|_| ())
    }
    pub(super) fn pending(&self, db: &Connection, statuses: &[String; 2]) -> Result<DataRows> {
        read_on_connection(
            db,
            &self.pending,
            &Self::call(&self.pending, &[&statuses[0], &statuses[1]]),
            RowPolicy::All,
            &[ColumnCodec::String; 3],
        )
    }
    pub(super) fn current(&self, db: &Connection, id: &str, path: &str) -> Result<DataRows> {
        read_on_connection(
            db,
            &self.current,
            &Self::call(&self.current, &[id, path]),
            RowPolicy::FirstOptional,
            &[ColumnCodec::String],
        )
    }
    pub(super) fn counts(&self, db: &Connection, statuses: &[String; 2]) -> Result<DataRows> {
        read_on_connection(
            db,
            &self.counts,
            &Self::call(&self.counts, &[&statuses[0], &statuses[1]]),
            RowPolicy::FirstRequired,
            &[ColumnCodec::I64; 2],
        )
    }
    pub(super) fn external(&self, db: &Connection, id: &str, path: &str) -> Result<()> {
        write_on_connection(db, &self.external, &Self::call(&self.external, &[id, path]))
            .map(|_| ())
    }
    pub(super) fn error(&self, db: &Connection, id: &str, path: &str, cause: &str) -> Result<()> {
        write_on_connection(
            db,
            &self.error,
            &Self::call(&self.error, &[id, path, cause]),
        )
        .map(|_| ())
    }
}
