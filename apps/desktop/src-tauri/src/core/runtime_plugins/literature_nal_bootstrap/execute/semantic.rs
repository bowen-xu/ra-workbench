use super::Execution;
use crate::{
    ai::embedding_settings,
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P},
    runtime_plugins::{
        literature_nal_bootstrap::{error::CaptureError, outcome::BootstrapProblem, rows::Row},
        native::SafeDatabaseError,
    },
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue as V},
    knowledge_snapshot::KnowledgeConcept,
    nal_bootstrap::*,
};

impl Execution<'_, '_> {
    pub(super) fn semantic(
        &mut self,
        prepared: &PreparedConceptV1,
        actual: &KnowledgeConcept,
        slots: &ConceptIdentitySlotsV1,
    ) -> Result<(), BootstrapProblem> {
        self.semantic_render_stop(&prepared.key)?;
        let resources = self.resources;
        let rows = self.read(
            BootstrapPhaseV1::RepresentationLookup,
            &resources.representation_lookup,
            vec![
                V::Text(actual.id.clone()),
                V::Integer(actual.revision),
                V::Text(actual.preferred_term.language.clone()),
                V::Text(self.input.probes.representation_render_version.clone()),
            ],
            P::FirstOptional,
            &[C::String],
        )?;
        let existing = optional_id(rows)
            .map_err(|error| self.problem(BootstrapPhaseV1::RepresentationLookup, error))?;
        let SourceProbe::Available {
            value: representation,
        } = &prepared.representation
        else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::RepresentationLookup,
                "representation-preparation",
            ));
        };
        if representation.language != actual.preferred_term.language
            || representation.render_version != self.input.probes.representation_render_version
        {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::RepresentationLookup,
                "representation-input-binding",
            ));
        }
        if existing.as_deref()
            != representation
                .existing
                .then_some(representation.id.as_str())
        {
            return Err(self.problem(
                BootstrapPhaseV1::RepresentationLookup,
                CaptureError::changed(None),
            ));
        }
        if existing.is_none() {
            if representation.id != slots.representation_id {
                return Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::RepresentationInsert,
                    "representation-id",
                ));
            }
            let args = vec![V::Text(actual.id.clone()), V::Integer(actual.revision)];
            self.write(
                BootstrapPhaseV1::RepresentationInsert,
                &resources.semantic_old_jobs_cancel,
                args.clone(),
            )?;
            self.write(
                BootstrapPhaseV1::RepresentationInsert,
                &resources.semantic_old_records_tombstone,
                args,
            )?;
            self.write(
                BootstrapPhaseV1::RepresentationInsert,
                &resources.representation_insert,
                vec![
                    V::Text(representation.id.clone()),
                    V::Text(actual.id.clone()),
                    V::Integer(actual.revision),
                    V::Text(representation.language.clone()),
                    V::Text(representation.render_version.clone()),
                    V::Text(representation.rendered_text.clone()),
                    V::Text(representation.input_sha256.clone()),
                ],
            )?;
        }
        self.captured
            .require_phase_in(self.db, BootstrapPhaseV1::EmbeddingSettings)
            .map_err(|error| self.problem(BootstrapPhaseV1::EmbeddingSettings, error))?;
        let configured = embedding_settings::configured_space_in(self.db)
            .map(|value| {
                value.map(|value| ConfiguredEmbeddingV1 {
                    region: value.region,
                    model_id: value.model_id,
                })
            })
            .map_err(|error| {
                self.problem(
                    BootstrapPhaseV1::EmbeddingSettings,
                    CaptureError::Owner(Box::new(SafeDatabaseError::sql(
                        "embedding_settings",
                        &error,
                    ))),
                )
            })?;
        let SourceProbe::Available { value: captured } = &self.input.source.configured_embedding
        else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::EmbeddingSettings,
                "configured-embedding",
            ));
        };
        if &configured != captured {
            return Err(self.problem(
                BootstrapPhaseV1::EmbeddingSettings,
                CaptureError::changed(None),
            ));
        }
        if configured.is_none() {
            if !matches!(
                prepared.semantic_queue,
                SourceProbe::Available { value: None }
            ) {
                return Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::EmbeddingSettings,
                    "no-embedding-queue",
                ));
            }
            return Ok(());
        }
        let SourceProbe::Available { value: Some(space) } = &self.input.probes.semantic_space
        else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::SpaceInsert,
                "semantic-space",
            ));
        };
        self.write(
            BootstrapPhaseV1::SpaceInsert,
            &resources.embedding_space_insert,
            vec![
                V::Text(space.id.clone()),
                V::Text(space.region.clone()),
                V::Text(space.model_id.clone()),
                V::Integer(space.dimension),
                V::Text(space.instruction_set_version.clone()),
            ],
        )?;
        let args = vec![
            V::Text(space.id.clone()),
            V::Text(self.input.probes.semantic_namespace.clone()),
        ];
        let active = self.read(
            BootstrapPhaseV1::ActiveGeneration,
            &resources.generation_active,
            args.clone(),
            P::FirstOptional,
            &[C::String, C::I64],
        )?;
        let active = active
            .rows
            .into_iter()
            .next()
            .map(|row| {
                let mut row = Row::from(row);
                Ok((row.text()?, row.integer()?))
            })
            .transpose()
            .map_err(|error| self.problem(BootstrapPhaseV1::ActiveGeneration, error))?;
        let next = if active.is_none() {
            let rows = self.read(
                BootstrapPhaseV1::NextGeneration,
                &resources.generation_next,
                args,
                P::FirstRequired,
                &[C::I64],
            )?;
            Some(
                required_integer(rows)
                    .map_err(|error| self.problem(BootstrapPhaseV1::NextGeneration, error))?,
            )
        } else {
            None
        };
        let SourceProbe::Available { value: generation } = &prepared.semantic_generation else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::GenerationInsert,
                "generation-preparation",
            ));
        };
        if generation.embedding_space_id != space.id {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::GenerationInsert,
                "generation-space",
            ));
        }
        match active {
            Some((id, number))
                if !generation.create && id == generation.id && number == generation.generation => {
            }
            None if generation.create && next == Some(generation.generation) => {
                if self.input.identities.generation_id.as_ref() != Some(&generation.id) {
                    return Err(BootstrapProblem::binding(
                        BootstrapPhaseV1::GenerationInsert,
                        "generation-id",
                    ));
                }
                self.write(
                    BootstrapPhaseV1::GenerationInsert,
                    &resources.generation_insert,
                    vec![
                        V::Text(generation.id.clone()),
                        V::Text(self.input.probes.semantic_namespace.clone()),
                        V::Text(space.id.clone()),
                        V::Integer(generation.generation),
                        V::Text(generation.relative_database_path.clone()),
                    ],
                )?;
            }
            _ => {
                return Err(self.problem(
                    BootstrapPhaseV1::ActiveGeneration,
                    CaptureError::changed(None),
                ));
            }
        }
        let rows = self.read(
            BootstrapPhaseV1::QueueLookup,
            &resources.embedding_job_lookup,
            vec![
                V::Text(representation.id.clone()),
                V::Text(space.id.clone()),
                V::Text(generation.id.clone()),
            ],
            P::FirstOptional,
            &[C::String],
        )?;
        let existing = optional_id(rows)
            .map_err(|error| self.problem(BootstrapPhaseV1::QueueLookup, error))?;
        let SourceProbe::Available { value: Some(queue) } = &prepared.semantic_queue else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::QueueLookup,
                "queue-preparation",
            ));
        };
        if queue.space != *space || queue.generation != *generation {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::QueueLookup,
                "queue-generation-binding",
            ));
        }
        match (&existing, &queue.job) {
            (Some(actual), QueueResolutionV1::Existing { job_id }) if actual == job_id => Ok(()),
            (
                None,
                QueueResolutionV1::Create {
                    record_id,
                    job_id,
                    row_id,
                    idempotency_key,
                },
            ) => {
                if record_id != &slots.record_id || job_id != &slots.job_id {
                    return Err(BootstrapProblem::binding(
                        BootstrapPhaseV1::RecordInsert,
                        "queue-identities",
                    ));
                }
                self.write(
                    BootstrapPhaseV1::RecordInsert,
                    &resources.embedding_record_insert,
                    vec![
                        V::Text(record_id.clone()),
                        V::Text(representation.id.clone()),
                        V::Text(space.id.clone()),
                        V::Text(generation.id.clone()),
                        V::Text(self.input.probes.semantic_namespace.clone()),
                        V::Text(row_id.clone()),
                    ],
                )?;
                self.write(
                    BootstrapPhaseV1::JobInsert,
                    &resources.embedding_job_insert,
                    vec![
                        V::Text(job_id.clone()),
                        V::Text(record_id.clone()),
                        V::Text(idempotency_key.clone()),
                    ],
                )?;
                Ok(())
            }
            _ => Err(self.problem(BootstrapPhaseV1::QueueLookup, CaptureError::changed(None))),
        }
    }

    fn semantic_render_stop(&self, key: &ConceptCallKeyV1) -> Result<(), BootstrapProblem> {
        if let Some(stop) = &self.plan.terminal
            && stop.phase == BootstrapPhaseV1::SemanticRender
            && stop.concept_key.as_ref() == Some(key)
        {
            return Err(match &stop.reason {
                BootstrapStopReasonV1::Rejected { reason } => {
                    self.concept_reject(stop.phase, *reason)
                }
                _ => BootstrapProblem::binding(stop.phase, "semantic-render-stop"),
            });
        }
        Ok(())
    }
}

fn optional_id(rows: DataRows) -> Result<Option<String>, CaptureError> {
    rows.rows
        .into_iter()
        .next()
        .map(|row| Row::from(row).text())
        .transpose()
}
fn required_integer(rows: DataRows) -> Result<i64, CaptureError> {
    rows.rows
        .into_iter()
        .next()
        .ok_or(CaptureError::Shape("required-row"))
        .and_then(|row| Row::from(row).integer())
}
