//! The original complete NAL load and codec order, with caller-owned reads.
use super::{CaptureError, capture::first, rows::Row};
use crate::{
    data_interface::plugin_resources::{ColumnCodec, RowPolicy},
    runtime_plugins::data::AcceptedDataResource,
};
use research_workbench_native_sdk::{
    data::DataRows,
    nal_annotation::NalAnnotation,
    nal_bootstrap::{BootstrapPhaseV1, RawNalStateV1, RawNalStatementV1},
    nal_state::{NalConcept, NalStatement},
};

pub(crate) struct NalLoadResources<'a> {
    pub(crate) revision: &'a AcceptedDataResource,
    pub(crate) concepts: &'a AcceptedDataResource,
    pub(crate) statements: &'a AcceptedDataResource,
    pub(crate) annotations: &'a AcceptedDataResource,
    pub(crate) rules: &'a AcceptedDataResource,
}

pub(crate) struct DecodedNalState {
    pub(crate) raw: RawNalStateV1,
    pub(crate) statements: Vec<NalStatement>,
}

#[derive(Debug)]
pub(crate) enum CompleteNalLoadError<E> {
    Read(E),
    Capture {
        phase: BootstrapPhaseV1,
        cause: CaptureError,
    },
    Structure(serde_json::Error),
}

/// No connection, guard, readset or prepared rule projection is owned here.
pub(crate) fn load_complete<E>(
    resources: NalLoadResources<'_>,
    mut read: impl FnMut(
        BootstrapPhaseV1,
        &AcceptedDataResource,
        RowPolicy,
        &[ColumnCodec],
    ) -> Result<DataRows, E>,
) -> Result<DecodedNalState, CompleteNalLoadError<E>> {
    use BootstrapPhaseV1 as Phase;
    use ColumnCodec as C;
    use RowPolicy as P;

    let revision = read(
        Phase::StateRevision,
        resources.revision,
        P::FirstRequired,
        &[C::I64],
    )
    .map_err(CompleteNalLoadError::Read)?;
    let revision = first(revision)
        .and_then(|mut row| row.integer())
        .map_err(|cause| CompleteNalLoadError::Capture {
            phase: Phase::StateRevision,
            cause,
        })?;
    let rows = read(
        Phase::StateConcepts,
        resources.concepts,
        P::All,
        &[C::String, C::String, C::String, C::I64, C::String],
    )
    .map_err(CompleteNalLoadError::Read)?;
    let concepts = rows
        .rows
        .into_iter()
        .map(|row| {
            let mut row = Row::from(row);
            Ok(NalConcept {
                id: row.text()?,
                label: row.text()?,
                explanation: row.text()?,
                revision: row.integer()?,
                concept_kind: row.text()?,
            })
        })
        .collect::<Result<Vec<_>, CaptureError>>()
        .map_err(|cause| CompleteNalLoadError::Capture {
            phase: Phase::StateConcepts,
            cause,
        })?;
    let rows = read(
        Phase::StateStatements,
        resources.statements,
        P::All,
        &[C::String, C::String, C::String, C::String],
    )
    .map_err(CompleteNalLoadError::Read)?;
    let raw_statements = rows
        .rows
        .into_iter()
        .map(|row| {
            let mut row = Row::from(row);
            Ok(RawNalStatementV1 {
                id: row.text()?,
                subject_json: row.text()?,
                predicate_json: row.text()?,
                origin: row.text()?,
            })
        })
        .collect::<Result<Vec<_>, CaptureError>>()
        .map_err(|cause| CompleteNalLoadError::Capture {
            phase: Phase::StateStatements,
            cause,
        })?;
    // Decode every primitive row first. Fact JSON must fail before annotation
    // or rule SQL, including valid JSON with an incompatible fact shape.
    let statements = raw_statements
        .iter()
        .map(|row| {
            Ok(NalStatement {
                id: row.id.clone(),
                subject: serde_json::from_str(&row.subject_json)?,
                predicate: serde_json::from_str(&row.predicate_json)?,
                origin: row.origin.clone(),
            })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()
        .map_err(CompleteNalLoadError::Structure)?;
    let rows = read(
        Phase::StateAnnotations,
        resources.annotations,
        P::All,
        &[C::String, C::String, C::String, C::String],
    )
    .map_err(CompleteNalLoadError::Read)?;
    let annotations = rows
        .rows
        .into_iter()
        .map(|row| {
            let mut row = Row::from(row);
            Ok(NalAnnotation {
                id: row.text()?,
                target_kind: row.text()?,
                target_id: row.text()?,
                note_page_id: row.text()?,
            })
        })
        .collect::<Result<Vec<_>, CaptureError>>()
        .map_err(|cause| CompleteNalLoadError::Capture {
            phase: Phase::StateAnnotations,
            cause,
        })?;
    let rows = read(Phase::StateRules, resources.rules, P::All, &[C::String])
        .map_err(CompleteNalLoadError::Read)?;
    let relation_concept_ids = rows
        .rows
        .into_iter()
        .map(|row| Row::from(row).text())
        .collect::<Result<Vec<_>, CaptureError>>()
        .map_err(|cause| CompleteNalLoadError::Capture {
            phase: Phase::StateRules,
            cause,
        })?;
    Ok(DecodedNalState {
        raw: RawNalStateV1 {
            revision,
            concepts,
            statements: raw_statements,
            annotations,
            relation_concept_ids,
        },
        statements,
    })
}
