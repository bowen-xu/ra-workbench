use crate::{
    ai::{
        paper_reading_contracts::SnapshotEvidenceRow,
        structured_resource_validation::{
            ExplicitReadingEvidence, ReadingValidationEvidence, SourceReadFailure,
        },
    },
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
        reading_evidence::ReadingEvidenceResources,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue,
};
use rusqlite::Connection;

#[derive(Clone)]
pub(crate) struct HbsReadingEvidenceResources {
    explicit: ReadingEvidenceResources,
    latest: AcceptedDataResource,
}
impl HbsReadingEvidenceResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let explicit = ReadingEvidenceResources::load(catalog, package)?;
        let text = DataField {
            data_type: DataType::Text,
            nullable: false,
        };
        let latest = AcceptedDataResource::load(
            catalog,
            package,
            "literature_library",
            "data/reading-evidence-validation-latest.v1.sql",
            DataResourceGrant {
                id: "literature.reading-evidence.validation-latest".into(),
                version: 1,
                sha256: "bfc8196efd5310654731f39829cf65e00c1d1eeba62225263923b65e17815e84".into(),
                access: DataAccess::Read,
                parameters: vec![text],
                columns: vec![text],
            },
        )?;
        Ok(Self { explicit, latest })
    }
    pub(crate) fn package_key(&self) -> &str {
        self.explicit.package_key()
    }
}
impl ExplicitReadingEvidence for HbsReadingEvidenceResources {
    fn snapshot_hash(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        self.explicit.snapshot_hash(db, snapshot, paper)
    }
    fn snapshot_evidence_rows(
        &self,
        db: &Connection,
        snapshot: &str,
    ) -> Result<Vec<SnapshotEvidenceRow>, SourceReadFailure> {
        self.explicit.snapshot_evidence_rows(db, snapshot)
    }
}
impl ReadingValidationEvidence for HbsReadingEvidenceResources {
    fn validation_latest_snapshot(
        &self,
        db: &Connection,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        let rows = read_on_connection(
            db,
            &self.latest,
            &DataCall {
                id: self.latest.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(paper.into())],
            },
            RowPolicy::FirstOptional,
            &[ColumnCodec::String],
        )
        .map_err(SourceReadFailure::Native)?;
        rows.rows
            .into_iter()
            .next()
            .map(|row| match row.as_slice() {
                [DataValue::Text(value)] => Ok(value.clone()),
                _ => Err(SourceReadFailure::Native(Box::new(PluginDataError {
                    code: "plugin_data_result_incompatible",
                    message: "冻结来源数据与固定契约不符。",
                    package_key: self.package_key().into(),
                    resource_id: self.latest.grant().id.clone(),
                    resource_version: 1,
                    phase: "read",
                    safe_cause: "reading_evidence_data_shape".into(),
                    sqlite_code: None,
                    sqlite_extended_code: None,
                    original_sql_failure: None,
                }))),
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests;
