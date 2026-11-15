//! Two plugin-owned snapshot statements executed on the original Core guard.
use crate::{
    ai::{
        paper_reading_contracts::SnapshotEvidenceRow,
        structured_resource_validation::{ExplicitReadingEvidence, SourceReadFailure},
    },
    data_interface::plugin_resources::{
        ColumnCodec as C, PluginDataError, RowPolicy as P, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue as V,
};
use rusqlite::Connection;

pub(crate) const CAPABILITY_ID: &str = "literature.reading-evidence";
const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};

#[derive(Clone)]
pub(crate) struct ReadingEvidenceResources {
    hash: AcceptedDataResource,
    rows: AcceptedDataResource,
}
impl ReadingEvidenceResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, sha: &str, parameters: Vec<DataField>, columns: Vec<DataField>| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/reading-evidence-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.reading-evidence.{name}"),
                    version: 1,
                    sha256: sha.into(),
                    access: DataAccess::Read,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            hash: load(
                "snapshot-hash",
                "9fd93259f0cb66e07ac7759be59697ea33c853cbf9a5df428d8fd15e868d5f7d",
                vec![TEXT, TEXT],
                vec![TEXT],
            )?,
            rows: load(
                "snapshot-rows",
                "17bdfcf4e74fe021b781f8bcdb48f8a35266f3e99f4f7c6c8af107a9b8388e6b",
                vec![TEXT],
                vec![TEXT, TEXT, TEXT, INTEGER, TEXT, TEXT],
            )?,
        })
    }
    pub(crate) fn package_key(&self) -> &str {
        self.hash.package_key()
    }
    fn shape(resource: &AcceptedDataResource) -> SourceReadFailure {
        SourceReadFailure::Native(Box::new(PluginDataError {
            code: "plugin_data_result_incompatible",
            message: "冻结来源数据与固定契约不符。",
            package_key: resource.package_key().into(),
            resource_id: resource.grant().id.clone(),
            resource_version: 1,
            phase: "read",
            safe_cause: "reading_evidence_data_shape".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        }))
    }
}
fn call(resource: &AcceptedDataResource, arguments: Vec<V>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: 1,
        arguments,
    }
}
impl ExplicitReadingEvidence for ReadingEvidenceResources {
    fn snapshot_hash(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        let rows = read_on_connection(
            db,
            &self.hash,
            &call(
                &self.hash,
                vec![V::Text(snapshot.into()), V::Text(paper.into())],
            ),
            P::FirstOptional,
            &[C::String],
        )
        .map_err(SourceReadFailure::Native)?;
        rows.rows
            .into_iter()
            .next()
            .map(|row| match row.as_slice() {
                [V::Text(value)] => Ok(value.clone()),
                _ => Err(Self::shape(&self.hash)),
            })
            .transpose()
    }
    fn snapshot_evidence_rows(
        &self,
        db: &Connection,
        snapshot: &str,
    ) -> Result<Vec<SnapshotEvidenceRow>, SourceReadFailure> {
        let rows = read_on_connection(
            db,
            &self.rows,
            &call(&self.rows, vec![V::Text(snapshot.into())]),
            P::All,
            &[
                C::String,
                C::String,
                C::String,
                C::I64,
                C::String,
                C::String,
            ],
        )
        .map_err(SourceReadFailure::Native)?;
        rows.rows
            .into_iter()
            .map(|row| match row.as_slice() {
                [
                    V::Text(id),
                    V::Text(attachment),
                    V::Text(sha),
                    V::Integer(page),
                    V::Text(text),
                    V::Text(current),
                ] => Ok((
                    id.clone(),
                    attachment.clone(),
                    sha.clone(),
                    *page,
                    text.clone(),
                    current.clone(),
                )),
                _ => Err(Self::shape(&self.rows)),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
