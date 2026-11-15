//! Shared fixed representation/settings/generation/queue sequence; policies remain package SQL.
use crate::{
    ai::embedding_settings,
    data_interface::plugin_resources::{
        ColumnCodec as C, RowPolicy as P, read_on_connection, write_in,
    },
    runtime_plugins::{
        data::AcceptedDataResource, literature_nal_bootstrap::CaptureError,
        native::SafeDatabaseError,
    },
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue as V},
    knowledge_snapshot::KnowledgeConcept,
    nal_bootstrap::BootstrapPhaseV1 as Phase,
};
use rusqlite::Transaction;
use uuid::Uuid;

pub(crate) struct SemanticFailure {
    pub(crate) phase: Phase,
    pub(crate) cause: CaptureError,
}
type Result<T> = std::result::Result<T, SemanticFailure>;
#[derive(Clone, Copy)]
pub(crate) struct SemanticResources<'a> {
    pub(crate) legacy_render_concept: &'a AcceptedDataResource,
    pub(crate) representation_lookup: &'a AcceptedDataResource,
    pub(crate) semantic_old_jobs_cancel: &'a AcceptedDataResource,
    pub(crate) semantic_old_records_tombstone: &'a AcceptedDataResource,
    pub(crate) representation_insert: &'a AcceptedDataResource,
    pub(crate) semantic_space_policy: &'a AcceptedDataResource,
    pub(crate) embedding_space_insert: &'a AcceptedDataResource,
    pub(crate) generation_active: &'a AcceptedDataResource,
    pub(crate) generation_next: &'a AcceptedDataResource,
    pub(crate) semantic_generation_path: &'a AcceptedDataResource,
    pub(crate) generation_insert: &'a AcceptedDataResource,
    pub(crate) embedding_job_lookup: &'a AcceptedDataResource,
    pub(crate) semantic_record_row_id: &'a AcceptedDataResource,
    pub(crate) embedding_record_insert: &'a AcceptedDataResource,
    pub(crate) semantic_job_idempotency: &'a AcceptedDataResource,
    pub(crate) embedding_job_insert: &'a AcceptedDataResource,
}
impl<'a> SemanticResources<'a> {
    pub(super) fn from_concepts(r: &'a super::ConceptResources) -> Self {
        Self {
            legacy_render_concept: r.resource("render"),
            representation_lookup: r.resource("representation-lookup"),
            semantic_old_jobs_cancel: r.resource("semantic-old-jobs-cancel"),
            semantic_old_records_tombstone: r.resource("semantic-old-records-tombstone"),
            representation_insert: r.resource("representation-insert"),
            semantic_space_policy: r.resource("space-policy"),
            embedding_space_insert: r.resource("embedding-space-insert"),
            generation_active: r.resource("generation-active"),
            generation_next: r.resource("generation-next"),
            semantic_generation_path: r.resource("generation-path"),
            generation_insert: r.resource("generation-insert"),
            embedding_job_lookup: r.resource("embedding-job-lookup"),
            semantic_record_row_id: r.resource("record-row-id"),
            embedding_record_insert: r.resource("embedding-record-insert"),
            semantic_job_idempotency: r.resource("job-idempotency"),
            embedding_job_insert: r.resource("embedding-job-insert"),
        }
    }
}
pub(crate) fn register(
    tx: &Transaction<'_>,
    resources: SemanticResources<'_>,
    actual: &KnowledgeConcept,
) -> Result<()> {
    SemanticIo { tx, resources }.semantic(actual)
}
struct SemanticIo<'a, 'tx> {
    tx: &'a Transaction<'tx>,
    resources: SemanticResources<'a>,
}
struct Row(std::vec::IntoIter<V>);
impl Row {
    fn from(row: Vec<V>) -> Self {
        Self(row.into_iter())
    }
    fn text(&mut self) -> std::result::Result<String, CaptureError> {
        match self.0.next() {
            Some(V::Text(v)) => Ok(v),
            _ => Err(CaptureError::Shape("semantic-text")),
        }
    }
    fn integer(&mut self) -> std::result::Result<i64, CaptureError> {
        match self.0.next() {
            Some(V::Integer(v)) => Ok(v),
            _ => Err(CaptureError::Shape("semantic-i64")),
        }
    }
}
impl SemanticIo<'_, '_> {
    fn problem(&self, phase: Phase, cause: CaptureError) -> SemanticFailure {
        SemanticFailure { phase, cause }
    }
    fn read(
        &self,
        phase: Phase,
        r: &AcceptedDataResource,
        args: Vec<V>,
        policy: P,
        codecs: &[C],
    ) -> Result<DataRows> {
        read_on_connection(self.tx, r, &super::resources::call(r, args), policy, codecs)
            .map_err(|e| self.problem(phase, CaptureError::Data(e)))
    }
    fn first(
        &self,
        phase: Phase,
        r: &AcceptedDataResource,
        args: Vec<V>,
        codecs: &[C],
    ) -> Result<Row> {
        self.read(phase, r, args, P::FirstRequired, codecs)?
            .rows
            .into_iter()
            .next()
            .map(Row::from)
            .ok_or_else(|| self.problem(phase, CaptureError::Shape("semantic-row")))
    }
    fn write(&self, phase: Phase, r: &AcceptedDataResource, args: Vec<V>) -> Result<u64> {
        write_in(self.tx, r, &super::resources::call(r, args))
            .map_err(|e| self.problem(phase, CaptureError::Data(e)))
    }
    fn encode(&self, phase: Phase, value: &impl serde::Serialize) -> Result<String> {
        serde_json::to_string(value)
            .map_err(|_| self.problem(phase, CaptureError::Shape("semantic-codec")))
    }
    fn text(&self, phase: Phase, r: &AcceptedDataResource, args: Vec<V>) -> Result<String> {
        self.first(phase, r, args, &[C::String])?
            .text()
            .map_err(|e| self.problem(phase, e))
    }
    fn optional(
        &self,
        phase: Phase,
        r: &AcceptedDataResource,
        args: Vec<V>,
    ) -> Result<Option<String>> {
        self.read(phase, r, args, P::FirstOptional, &[C::String])?
            .rows
            .into_iter()
            .next()
            .map(|v| Row::from(v).text())
            .transpose()
            .map_err(|e| self.problem(phase, e))
    }
}
fn text(value: &str) -> V {
    V::Text(value.into())
}
impl SemanticIo<'_, '_> {
    fn semantic(&self, actual: &KnowledgeConcept) -> Result<()> {
        let mut rendered = self.first(
            Phase::SemanticRender,
            self.resources.legacy_render_concept,
            vec![V::Text(self.encode(Phase::SemanticRender, actual)?)],
            &[C::String; 3],
        )?;
        let (body, hash, version) =
            (|| Ok::<_, CaptureError>((rendered.text()?, rendered.text()?, rendered.text()?)))()
                .map_err(|e| self.problem(Phase::SemanticRender, e))?;
        let representation = self.optional(
            Phase::RepresentationLookup,
            self.resources.representation_lookup,
            vec![
                text(&actual.id),
                V::Integer(actual.revision),
                text(&actual.preferred_term.language),
                text(&version),
            ],
        )?;
        let representation = if let Some(id) = representation {
            id
        } else {
            let id = Uuid::new_v4().to_string();
            let args = vec![text(&actual.id), V::Integer(actual.revision)];
            self.write(
                Phase::RepresentationInsert,
                self.resources.semantic_old_jobs_cancel,
                args.clone(),
            )?;
            self.write(
                Phase::RepresentationInsert,
                self.resources.semantic_old_records_tombstone,
                args,
            )?;
            self.write(
                Phase::RepresentationInsert,
                self.resources.representation_insert,
                vec![
                    text(&id),
                    text(&actual.id),
                    V::Integer(actual.revision),
                    text(&actual.preferred_term.language),
                    V::Text(version),
                    V::Text(body),
                    V::Text(hash),
                ],
            )?;
            id
        };
        let configured = embedding_settings::configured_space_in(self.tx).map_err(|e| {
            self.problem(
                Phase::EmbeddingSettings,
                CaptureError::Owner(Box::new(SafeDatabaseError::sql("embedding_settings", &e))),
            )
        })?;
        let Some(configured) = configured else {
            return Ok(());
        };
        let mut space = self.first(
            Phase::SpaceInsert,
            self.resources.semantic_space_policy,
            vec![V::Text(configured.region), V::Text(configured.model_id)],
            &[
                C::String,
                C::String,
                C::String,
                C::I64,
                C::String,
                C::String,
            ],
        )?;
        let (space_id, region, model, dimension, instructions, namespace) = (|| {
            Ok::<_, CaptureError>((
                space.text()?,
                space.text()?,
                space.text()?,
                space.integer()?,
                space.text()?,
                space.text()?,
            ))
        })()
        .map_err(|e| self.problem(Phase::SpaceInsert, e))?;
        self.write(
            Phase::SpaceInsert,
            self.resources.embedding_space_insert,
            vec![
                text(&space_id),
                V::Text(region),
                V::Text(model),
                V::Integer(dimension),
                V::Text(instructions),
            ],
        )?;
        let active = self.read(
            Phase::ActiveGeneration,
            self.resources.generation_active,
            vec![text(&space_id), text(&namespace)],
            P::FirstOptional,
            &[C::String, C::I64],
        )?;
        let (generation_id, generation) = if let Some(row) = active.rows.into_iter().next() {
            let mut row = Row::from(row);
            (|| Ok::<_, CaptureError>((row.text()?, row.integer()?)))()
                .map_err(|e| self.problem(Phase::ActiveGeneration, e))?
        } else {
            let generation = self
                .first(
                    Phase::NextGeneration,
                    self.resources.generation_next,
                    vec![text(&space_id), text(&namespace)],
                    &[C::I64],
                )?
                .integer()
                .map_err(|e| self.problem(Phase::NextGeneration, e))?;
            let id = Uuid::new_v4().to_string();
            let path = self.text(
                Phase::GenerationInsert,
                self.resources.semantic_generation_path,
                vec![text(&space_id), V::Integer(generation)],
            )?;
            self.write(
                Phase::GenerationInsert,
                self.resources.generation_insert,
                vec![
                    text(&id),
                    text(&namespace),
                    text(&space_id),
                    V::Integer(generation),
                    V::Text(path),
                ],
            )?;
            (id, generation)
        };
        if self
            .optional(
                Phase::QueueLookup,
                self.resources.embedding_job_lookup,
                vec![text(&representation), text(&space_id), text(&generation_id)],
            )?
            .is_some()
        {
            return Ok(());
        }
        let record_id = Uuid::new_v4().to_string();
        let job_id = Uuid::new_v4().to_string();
        let row_id = self.text(
            Phase::RecordInsert,
            self.resources.semantic_record_row_id,
            vec![
                text(&representation),
                text(&space_id),
                V::Integer(generation),
            ],
        )?;
        self.write(
            Phase::RecordInsert,
            self.resources.embedding_record_insert,
            vec![
                text(&record_id),
                V::Text(representation),
                V::Text(space_id),
                V::Text(generation_id),
                V::Text(namespace),
                V::Text(row_id),
            ],
        )?;
        // The idempotency expression is deliberately after the record INSERT:
        // an earlier write failure must still win over any later expression.
        let idempotency = self.text(
            Phase::JobInsert,
            self.resources.semantic_job_idempotency,
            vec![text(&record_id)],
        )?;
        self.write(
            Phase::JobInsert,
            self.resources.embedding_job_insert,
            vec![V::Text(job_id), V::Text(record_id), V::Text(idempotency)],
        )?;
        Ok(())
    }
}
