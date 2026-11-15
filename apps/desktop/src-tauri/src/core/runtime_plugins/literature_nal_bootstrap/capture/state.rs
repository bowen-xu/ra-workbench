use super::{Context, first, texts};
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P},
    runtime_plugins::literature_nal_bootstrap::{
        error::Result, resources::BootstrapResources, rows::Row,
    },
};
use research_workbench_native_sdk::{
    nal_annotation::NalAnnotation, nal_bootstrap::*, nal_state::NalConcept,
};

pub(super) fn capture(
    resources: &BootstrapResources,
    c: &mut Context<'_>,
) -> Result<RawNalStateV1> {
    let revision = c.read(
        BootstrapPhaseV1::StateRevision,
        &resources.state_revision,
        vec![],
        P::FirstRequired,
        &[C::I64],
        |rows| first(rows)?.integer(),
    )?;
    let concepts = c.read(
        BootstrapPhaseV1::StateConcepts,
        &resources.state_concepts,
        vec![],
        P::All,
        &[C::String, C::String, C::String, C::I64, C::String],
        |rows| {
            rows.rows
                .into_iter()
                .map(|row| {
                    let mut r = Row::from(row);
                    Ok(NalConcept {
                        id: r.text()?,
                        label: r.text()?,
                        explanation: r.text()?,
                        revision: r.integer()?,
                        concept_kind: r.text()?,
                    })
                })
                .collect()
        },
    )?;
    // Deliberately do not parse fact JSON: the original cleanup/final load must
    // still decode it after all earlier writes and before later annotation SQL.
    let statements = c.read(
        BootstrapPhaseV1::StateStatements,
        &resources.state_statements,
        vec![],
        P::All,
        &[C::String, C::String, C::String, C::String],
        |rows| {
            rows.rows
                .into_iter()
                .map(|row| {
                    let mut r = Row::from(row);
                    Ok(RawNalStatementV1 {
                        id: r.text()?,
                        subject_json: r.text()?,
                        predicate_json: r.text()?,
                        origin: r.text()?,
                    })
                })
                .collect()
        },
    )?;
    let annotations = c.read(
        BootstrapPhaseV1::StateAnnotations,
        &resources.state_annotations,
        vec![],
        P::All,
        &[C::String, C::String, C::String, C::String],
        |rows| {
            rows.rows
                .into_iter()
                .map(|row| {
                    let mut r = Row::from(row);
                    Ok(NalAnnotation {
                        id: r.text()?,
                        target_kind: r.text()?,
                        target_id: r.text()?,
                        note_page_id: r.text()?,
                    })
                })
                .collect()
        },
    )?;
    let relation_concept_ids = c.read(
        BootstrapPhaseV1::StateRules,
        &resources.state_rules,
        vec![],
        P::All,
        &[C::String],
        texts,
    )?;
    Ok(RawNalStateV1 {
        revision,
        concepts,
        statements,
        annotations,
        relation_concept_ids,
    })
}
