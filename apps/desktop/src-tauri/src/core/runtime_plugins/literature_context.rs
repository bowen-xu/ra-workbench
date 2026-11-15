//! Finite literature SQL executed by Core on its caller's original connection.
use crate::{
    ai::{
        error::AiError,
        paper_reading_contracts::*,
        reading_support::PaperReadingFacts,
        structured_resource_validation::{ExplicitReadingEvidence, SourceReadFailure},
    },
    data_interface::plugin_resources::{
        ColumnCodec as C, PluginDataError, RowPolicy as P, read_on_connection,
    },
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
        reading_evidence::ReadingEvidenceResources,
    },
};
use research_workbench_native_sdk::{
    data::{DataCall, DataRows, DataValue as V},
    paper_context::{PaperContextBuildInputV1, SnapshotProjectionV1},
};
use rusqlite::Connection;
mod build;
mod failure;
pub(crate) use failure::{ai as source_failure, sql as sql_failure};
pub(crate) mod integrity;
pub(crate) mod preparation;
pub(crate) mod preview;
mod publication;
mod snapshot;
mod specs;
use specs::Resource as R;
pub(crate) type ContextFailure = Box<BoundaryFailure<AiError>>;
pub(crate) type ContextResult<T> = Result<T, ContextFailure>;
#[derive(Clone)]
pub(crate) struct PaperContextResources {
    resources: Vec<AcceptedDataResource>,
    evidence: ReadingEvidenceResources,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuildCapture {
    pub(crate) input: PaperContextBuildInputV1,
    pub(crate) witness: BuildWitness,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuildWitness {
    resolved: Vec<Vec<V>>,
    parse_sources: Vec<Option<String>>,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SnapshotCapture {
    pub(crate) existing: Option<PaperContextSnapshot>,
    pub(crate) witness: SnapshotWitness,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SnapshotWitness {
    hash_rows: Vec<Vec<V>>,
    header: Vec<Vec<V>>,
    attachments: Vec<Vec<V>>,
    items: Vec<Vec<V>>,
}
impl PaperContextResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            resources: specs::load(catalog, package)?,
            evidence: ReadingEvidenceResources::load(catalog, package)?,
        })
    }
    pub(crate) fn package_key(&self) -> &str {
        self.evidence.package_key()
    }
    fn resource(&self, id: R) -> &AcceptedDataResource {
        &self.resources[id as usize]
    }
    fn call(&self, id: R, arguments: Vec<V>) -> DataCall {
        DataCall {
            id: self.resource(id).grant().id.clone(),
            version: 1,
            arguments,
        }
    }
    fn read(&self, db: &Connection, id: R, args: Vec<V>, policy: P) -> ContextResult<DataRows> {
        read_on_connection(
            db,
            self.resource(id),
            &self.call(id, args),
            policy,
            id.spec().codecs,
        )
        .map_err(failure::data)
    }
    fn shape(&self, id: R) -> ContextFailure {
        failure::data(Box::new(PluginDataError {
            code: "plugin_data_result_incompatible",
            message: "论文上下文数据与固定契约不符。",
            package_key: self.package_key().into(),
            resource_id: self.resource(id).grant().id.clone(),
            resource_version: 1,
            phase: "read",
            safe_cause: "paper_context_data_shape".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        }))
    }
    pub(crate) fn frozen_belongs(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> ContextResult<bool> {
        let rows = self.read(
            db,
            R::SnapshotMembership,
            vec![text(snapshot), text(paper)],
            P::FirstRequired,
        )?;
        match rows.rows.as_slice() {
            [row] => match row.as_slice() {
                [V::Integer(value)] => Ok(*value != 0),
                _ => Err(self.shape(R::SnapshotMembership)),
            },
            _ => Err(self.shape(R::SnapshotMembership)),
        }
    }
    pub(crate) fn item_geometry(
        &self,
        db: &Connection,
        snapshot: &str,
        item: &str,
    ) -> ContextResult<Option<SnapshotItemGeometry>> {
        let rows = self.read(
            db,
            R::SnapshotItemGeometry,
            vec![text(snapshot), text(item)],
            P::FirstOptional,
        )?;
        rows.rows
            .into_iter()
            .next()
            .map(|row| match row.as_slice() {
                [
                    V::Text(parse),
                    V::Text(raw),
                    V::Text(kind),
                    V::Text(regions),
                ] => Ok((parse.clone(), raw.clone(), kind.clone(), regions.clone())),
                _ => Err(self.shape(R::SnapshotItemGeometry)),
            })
            .transpose()
    }
}
fn text(value: &str) -> V {
    V::Text(value.into())
}
impl ExplicitReadingEvidence for PaperContextResources {
    fn snapshot_hash(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        self.evidence.snapshot_hash(db, snapshot, paper)
    }
    fn snapshot_evidence_rows(
        &self,
        db: &Connection,
        snapshot: &str,
    ) -> Result<Vec<SnapshotEvidenceRow>, SourceReadFailure> {
        self.evidence.snapshot_evidence_rows(db, snapshot)
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_fixture;
