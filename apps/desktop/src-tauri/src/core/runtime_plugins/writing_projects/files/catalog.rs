//! Original writing-note projection statements in the caller's file-index tx.
use super::super::{data_failure, failure};
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
        native::NativeResult,
    },
    scoped_text_files::legacy::sha256,
};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataResourceGrant, DataValue},
    writing_file::{FileSavePlan, WritingNoteCatalogProjection},
};
use rusqlite::Transaction;

pub(super) struct CatalogResources {
    source_hash: AcceptedDataResource,
    ready: AcceptedDataResource,
    body: AcceptedDataResource,
    error: AcceptedDataResource,
    delete_body: AcceptedDataResource,
}

pub(super) struct CatalogDocument<'a> {
    pub manuscript_id: &'a str,
    pub path: &'a str,
    pub content: &'a str,
    pub hash: &'a str,
}

impl CatalogResources {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
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
            source_hash: load(
                "note-source-hash",
                "7916d5daaeecf6e1da85a165347a56dfe1aad9ef63f8c865701c3909f46f89e3",
                DataAccess::Read,
                vec![text; 2],
                vec![text],
            )?,
            ready: load(
                "note-ready",
                "cd4aa36cecd60059e9027b36b1256e943f12ed219b6404ad161b72cbdf39248b",
                DataAccess::Write,
                vec![text; 3],
                vec![],
            )?,
            body: load(
                "note-body-upsert",
                "817913de115bd9537c8004ddeba45a1c9bfc621e68b0f2af1bd42446b08095c9",
                DataAccess::Write,
                vec![text; 4],
                vec![],
            )?,
            error: load(
                "note-error",
                "660da9ab44a73f2f48c1a25a7ce6f2062d88ca1dbabc25a53d84ae291ec30640",
                DataAccess::Write,
                vec![text; 3],
                vec![],
            )?,
            delete_body: load(
                "note-body-delete",
                "81925182ac5c1fc846c4ae233b4208cab16d246ca11cd18d64fbcf50aa46ffbe",
                DataAccess::Write,
                vec![text; 2],
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

    pub(super) fn record(
        &self,
        tx: &Transaction<'_>,
        plan: &FileSavePlan,
        hash: &str,
    ) -> NativeResult<()> {
        let request = &plan.preparation.request;
        let id = request.manuscript_id.trim();
        let path = request.relative_path.trim();
        self.record_projection(
            tx,
            CatalogDocument {
                manuscript_id: id,
                path,
                content: &request.content,
                hash,
            },
            &plan.catalog,
            &plan.duplicate_identity_cause,
        )
    }

    pub(super) fn record_projection(
        &self,
        tx: &Transaction<'_>,
        document: CatalogDocument<'_>,
        projection: &WritingNoteCatalogProjection,
        duplicate_identity_cause: &str,
    ) -> NativeResult<()> {
        if matches!(projection, WritingNoteCatalogProjection::Ignored) {
            return Ok(());
        }
        let CatalogDocument {
            manuscript_id: id,
            path,
            content,
            hash,
        } = document;
        let current = read_on_connection(
            tx,
            &self.source_hash,
            &Self::call(&self.source_hash, &[id, path]),
            RowPolicy::FirstOptional,
            &[ColumnCodec::String],
        )
        .map_err(data_failure)?;
        if current.rows != [vec![DataValue::Text(hash.into())]]
            || sha256(content.as_bytes()) != hash
        {
            return Err(failure(
                "file_conflict",
                "文件已在其他位置被修改，请重新载入后再保存",
                self.source_hash.package_key(),
            ));
        }
        let cause = match projection {
            WritingNoteCatalogProjection::Ready {
                note_id,
                text,
                folded_text,
            } => {
                match write_on_connection(
                    tx,
                    &self.ready,
                    &Self::call(&self.ready, &[id, path, note_id]),
                ) {
                    Ok(_) => {
                        write_on_connection(
                            tx,
                            &self.body,
                            &Self::call(&self.body, &[id, path, text, folded_text]),
                        )
                        .map_err(data_failure)?;
                        None
                    }
                    Err(cause)
                        if cause.sqlite_extended_code
                            == Some(rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE) =>
                    {
                        Some(duplicate_identity_cause)
                    }
                    Err(cause) => return Err(data_failure(cause)),
                }
            }
            WritingNoteCatalogProjection::Invalid { cause } => Some(cause.as_str()),
            WritingNoteCatalogProjection::Ignored => None,
        };
        if let Some(cause) = cause {
            write_on_connection(
                tx,
                &self.error,
                &Self::call(&self.error, &[id, path, cause]),
            )
            .map_err(data_failure)?;
            write_on_connection(
                tx,
                &self.delete_body,
                &Self::call(&self.delete_body, &[id, path]),
            )
            .map_err(data_failure)?;
        }
        Ok(())
    }
}
