//! Three fixed metadata reads on the caller's original Core connection.
use crate::{
    agent::paper_metadata::PaperMetadataError,
    ai::structured_resource_validation::{Failure, FailureCause},
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    context_metadata::ContextMetadataInputV1,
    data::{DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue},
};
use rusqlite::Connection;

const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
type Result<T> = std::result::Result<T, Failure<PaperMetadataError>>;

#[derive(Clone)]
pub(crate) struct ContextMetadataResources {
    header: AcceptedDataResource,
    creators: AcceptedDataResource,
    identifiers: AcceptedDataResource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ContextMetadataCapture {
    pub(crate) input: ContextMetadataInputV1,
    pub(crate) witness: ContextMetadataWitness,
}

/// Raw successful SQL values distinguish equal JSON with different stored bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextMetadataWitness {
    header: String,
    creators: Vec<[String; 2]>,
    identifiers: Vec<[String; 2]>,
}

impl ContextMetadataResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let load = |name: &str, sha: &str, columns: Vec<DataField>| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/context-metadata-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.context-metadata.{name}"),
                    version: 1,
                    sha256: sha.into(),
                    access: DataAccess::Read,
                    parameters: vec![TEXT],
                    columns,
                },
            )
        };
        Ok(Self {
            header: load(
                "header",
                "b34406f491c6bbf0dc5da5a9eb31f6b37409b8de8caaa0f9319bf125ecb1a784",
                vec![TEXT],
            )?,
            creators: load(
                "creators",
                "9ac0b30cc86ae93f1e42f82b9e123e122cac5fce8c4fd972c4d0244f999c9294",
                vec![TEXT, TEXT],
            )?,
            identifiers: load(
                "identifiers",
                "aa4dd69c737cf0b24ef022da422207c9782301b06afb1f8349758c9a66171ed1",
                vec![TEXT, TEXT],
            )?,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        self.header.package_key()
    }

    pub(crate) fn capture(&self, db: &Connection, paper: &str) -> Result<ContextMetadataCapture> {
        let rows = read_on_connection(
            db,
            &self.header,
            &call(&self.header, paper),
            RowPolicy::FirstRequired,
            &[ColumnCodec::String],
        )
        .map_err(|error| data_failure(PaperMetadataError::SourceUnavailable, error))?;
        let header = match rows.rows.into_iter().next().as_deref() {
            Some([DataValue::Text(value)]) => value.clone(),
            _ => return Err(shape(&self.header, PaperMetadataError::SourceUnavailable)),
        };
        let base = serde_json::from_str(&header)
            .map_err(|_| Failure::original(PaperMetadataError::Unavailable))?;
        let creators = pairs(&self.creators, db, paper)?;
        let identifiers = pairs(&self.identifiers, db, paper)?;
        Ok(ContextMetadataCapture {
            input: ContextMetadataInputV1 {
                paper_id: paper.into(),
                base,
                creators: creators.clone(),
                identifiers: identifiers.clone(),
            },
            witness: ContextMetadataWitness {
                header,
                creators,
                identifiers,
            },
        })
    }
}

fn call(resource: &AcceptedDataResource, paper: &str) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: 1,
        arguments: vec![DataValue::Text(paper.into())],
    }
}

fn pairs(
    resource: &AcceptedDataResource,
    db: &Connection,
    paper: &str,
) -> Result<Vec<[String; 2]>> {
    let rows = read_on_connection(
        db,
        resource,
        &call(resource, paper),
        RowPolicy::All,
        &[ColumnCodec::String, ColumnCodec::String],
    )
    .map_err(|error| data_failure(PaperMetadataError::Unavailable, error))?;
    rows.rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [DataValue::Text(first), DataValue::Text(second)] => {
                Ok([first.clone(), second.clone()])
            }
            _ => Err(shape(resource, PaperMetadataError::Unavailable)),
        })
        .collect()
}

fn data_failure(
    error: PaperMetadataError,
    cause: Box<PluginDataError>,
) -> Failure<PaperMetadataError> {
    Failure {
        error,
        cause: FailureCause::SourceData(cause),
    }
}

fn shape(
    resource: &AcceptedDataResource,
    error: PaperMetadataError,
) -> Failure<PaperMetadataError> {
    data_failure(
        error,
        Box::new(PluginDataError {
            code: "plugin_data_result_incompatible",
            message: "文献上下文元数据与固定契约不符。",
            package_key: resource.package_key().into(),
            resource_id: resource.grant().id.clone(),
            resource_version: 1,
            phase: "read",
            safe_cause: "context_metadata_data_shape".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        }),
    )
}

#[cfg(test)]
mod tests;
