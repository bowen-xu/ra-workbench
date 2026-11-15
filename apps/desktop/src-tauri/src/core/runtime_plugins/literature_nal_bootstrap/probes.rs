use super::{
    capture::{Context, PreparedBootstrapSource, first, probe, skipped},
    error::{CaptureError, Result},
    metadata::{TERM_CODECS, decode_term},
    readset::ReadSet,
    resources::BootstrapResources,
    rows::Row,
};
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P},
    runtime_plugins::data::AcceptedDataResource,
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue},
    nal_bootstrap::*,
};
use rusqlite::{Connection, Transaction};
use std::collections::HashSet;

pub(crate) struct PreparedBootstrapFacts {
    pub(crate) source: BootstrapSourceV1,
    pub(crate) facts: BootstrapProbeFactsV1,
    reads: ReadSet,
}

impl PreparedBootstrapFacts {
    pub(super) fn recheck_in(&self, tx: &Transaction<'_>) -> Result<()> {
        self.reads.recheck_in(tx)
    }
    pub(super) fn require_phase_in(&self, db: &Connection, phase: BootstrapPhaseV1) -> Result<()> {
        self.reads.require_phase_in(db, phase)
    }
    pub(super) fn require_probe_in(
        &self,
        db: &Connection,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: &[DataValue],
    ) -> Result<()> {
        self.reads.require_probe_in(db, phase, resource, arguments)
    }
    pub(super) fn check_source_rows(
        &self,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: &[DataValue],
        actual: &DataRows,
    ) -> Result<()> {
        self.reads
            .check_source_rows(phase, resource, arguments, actual)
    }
}

impl BootstrapResources {
    /// The caller owns one short read transaction. Recapture every R0 source in
    /// that same transaction before derived probes, then release it before native.
    pub(crate) fn capture_probes_in(
        &self,
        tx: &Transaction<'_>,
        previous: &PreparedBootstrapSource,
        preparation: &BootstrapProbePreparationV1,
        identities: &BootstrapIdentitiesV1,
    ) -> Result<PreparedBootstrapFacts> {
        let mut coherent = self.capture_source(tx);
        previous.same_capture(&coherent)?;
        validate_bindings(&coherent.source, preparation, identities)?;
        let mut c = Context {
            db: tx,
            reads: &mut coherent.reads,
        };
        let mut terms = Vec::with_capacity(preparation.term_keys.len());
        for key in &preparation.term_keys {
            let result = c.read(
                BootstrapPhaseV1::TermLookup,
                &self.term_lookup,
                vec![
                    DataValue::Text(key.normalized_text.clone()),
                    DataValue::Text(key.language.clone()),
                ],
                P::FirstOptional,
                &[C::String],
                |rows| {
                    rows.rows
                        .into_iter()
                        .next()
                        .map(|r| Row::from(r).text())
                        .transpose()
                },
            );
            let (result, failure_phase) = match result {
                Ok(Some(id)) => {
                    let result = c
                        .read(
                            BootstrapPhaseV1::TermReadback,
                            &self.term_read,
                            vec![DataValue::Text(id)],
                            P::FirstOptional,
                            TERM_CODECS,
                            decode_term,
                        )
                        .map(Some);
                    let phase = result
                        .as_ref()
                        .err()
                        .map(|_| BootstrapPhaseV1::TermReadback);
                    (result, phase)
                }
                Ok(None) => (Ok(None), None),
                Err(error) => (Err(error), Some(BootstrapPhaseV1::TermLookup)),
            };
            terms.push(TermLookupFactV1 {
                key: key.clone(),
                result: probe(result),
                failure_phase,
            });
        }
        let (generation, next_generation) = match &preparation.semantic_space {
            SourceProbe::Available { value: Some(space) } => {
                let args = vec![
                    DataValue::Text(space.id.clone()),
                    DataValue::Text(preparation.semantic_namespace.clone()),
                ];
                let active = c.read(
                    BootstrapPhaseV1::ActiveGeneration,
                    &self.generation_active,
                    args.clone(),
                    P::FirstOptional,
                    &[C::String, C::I64],
                    |rows| {
                        rows.rows
                            .into_iter()
                            .next()
                            .map(|row| {
                                let mut row = Row::from(row);
                                Ok(ExistingGenerationV1 {
                                    id: row.text()?,
                                    embedding_space_id: space.id.clone(),
                                    generation: row.integer()?,
                                })
                            })
                            .transpose()
                    },
                );
                let next = if matches!(&active, Ok(None)) {
                    probe(c.read(
                        BootstrapPhaseV1::NextGeneration,
                        &self.generation_next,
                        args,
                        P::FirstRequired,
                        &[C::I64],
                        |rows| first(rows)?.integer(),
                    ))
                } else {
                    skipped()
                };
                (probe(active), next)
            }
            _ => (skipped(), skipped()),
        };
        let mut concepts = Vec::with_capacity(preparation.concepts.len());
        for (intent, slots) in preparation.concepts.iter().zip(&identities.concepts) {
            let language = language(intent, &terms)?;
            let representation = if let Some(language) = language {
                probe(c.read(
                    BootstrapPhaseV1::RepresentationLookup,
                    &self.representation_lookup,
                    vec![
                        DataValue::Text(slots.concept_id.clone()),
                        DataValue::Integer(1),
                        DataValue::Text(language),
                        DataValue::Text(preparation.representation_render_version.clone()),
                    ],
                    P::FirstOptional,
                    &[C::String],
                    |rows| {
                        rows.rows
                            .into_iter()
                            .next()
                            .map(|r| Row::from(r).text())
                            .transpose()
                    },
                ))
            } else {
                skipped()
            };
            let queue_job = capture_job(
                self,
                &mut c,
                preparation,
                identities,
                slots,
                (&generation, &next_generation),
                &representation,
            );
            concepts.push(ConceptSemanticFactV1 {
                key: intent.key.clone(),
                representation,
                queue_job: queue_job?,
            });
        }
        Ok(PreparedBootstrapFacts {
            source: coherent.source,
            facts: BootstrapProbeFactsV1 {
                terms,
                generation,
                next_generation,
                concepts,
            },
            reads: coherent.reads,
        })
    }
}

fn language(intent: &ConceptIntentV1, facts: &[TermLookupFactV1]) -> Result<Option<String>> {
    let TermValidationV1::Prepared {
        normalized_text,
        language,
        ..
    } = &intent.term
    else {
        return Ok(None);
    };
    let key = TermLookupKeyV1 {
        normalized_text: normalized_text.clone(),
        language: language.clone(),
    };
    let fact = facts
        .iter()
        .find(|fact| fact.key == key)
        .ok_or(CaptureError::Shape("term-fact-binding"))?;
    Ok(match &fact.result {
        SourceProbe::Available { value: Some(term) } => Some(term.language.clone()),
        SourceProbe::Available { value: None } => Some(language.clone()),
        _ => None,
    })
}

fn capture_job(
    resources: &BootstrapResources,
    c: &mut Context<'_>,
    preparation: &BootstrapProbePreparationV1,
    identities: &BootstrapIdentitiesV1,
    slots: &ConceptIdentitySlotsV1,
    generation: (
        &SourceProbe<Option<ExistingGenerationV1>>,
        &SourceProbe<i64>,
    ),
    representation: &SourceProbe<Option<String>>,
) -> Result<SourceProbe<Option<String>>> {
    let SourceProbe::Available { value: Some(space) } = &preparation.semantic_space else {
        return Ok(skipped());
    };
    let (generation, next) = generation;
    let generation_id = match generation {
        SourceProbe::Available {
            value: Some(generation),
        } => &generation.id,
        SourceProbe::Available { value: None } if matches!(next, SourceProbe::Available { .. }) => {
            identities
                .generation_id
                .as_ref()
                .ok_or(CaptureError::Shape("generation-slot-binding"))?
        }
        _ => return Ok(skipped()),
    };
    let representation_id = match representation {
        SourceProbe::Available { value: Some(id) } => id,
        SourceProbe::Available { value: None } => &slots.representation_id,
        _ => return Ok(skipped()),
    };
    Ok(probe(c.read(
        BootstrapPhaseV1::QueueLookup,
        &resources.embedding_job_lookup,
        vec![
            DataValue::Text(representation_id.clone()),
            DataValue::Text(space.id.clone()),
            DataValue::Text(generation_id.clone()),
        ],
        P::FirstOptional,
        &[C::String],
        |rows| {
            rows.rows
                .into_iter()
                .next()
                .map(|row| Row::from(row).text())
                .transpose()
        },
    )))
}

fn validate_bindings(
    source: &BootstrapSourceV1,
    preparation: &BootstrapProbePreparationV1,
    identities: &BootstrapIdentitiesV1,
) -> Result<()> {
    if preparation.concepts.len() != identities.concepts.len()
        || preparation.statement_count != identities.statement_ids.len()
    {
        return Err(CaptureError::Shape("identity-slot-count"));
    }
    let mut seen = HashSet::new();
    let mut term_keys = Vec::new();
    for (concept, slots) in preparation.concepts.iter().zip(&identities.concepts) {
        if concept.key != slots.key || !seen.insert(&concept.key) {
            return Err(CaptureError::Shape("concept-slot-binding"));
        }
        match &concept.key {
            ConceptCallKeyV1::Collection { source_id } => {
                let bound = match &source.collections {
                    SourceProbe::Available { value } => {
                        value.iter().any(|row| &row.id == source_id)
                    }
                    _ => false,
                };
                if !bound {
                    return Err(CaptureError::Shape("collection-source-binding"));
                }
            }
            ConceptCallKeyV1::TypePredicate { source_id } => {
                let bound = match &source.relation_types {
                    SourceProbe::Available { value } => {
                        value.iter().any(|row| &row.id == source_id)
                    }
                    _ => false,
                };
                if !bound {
                    return Err(CaptureError::Shape("relation-type-source-binding"));
                }
            }
            _ => {}
        }
        if let TermValidationV1::Prepared {
            normalized_text,
            language,
            ..
        } = &concept.term
        {
            let key = TermLookupKeyV1 {
                normalized_text: normalized_text.clone(),
                language: language.clone(),
            };
            if !term_keys.contains(&key) {
                term_keys.push(key);
            }
        }
    }
    if term_keys != preparation.term_keys {
        return Err(CaptureError::Shape("term-probe-binding"));
    }
    Ok(())
}
