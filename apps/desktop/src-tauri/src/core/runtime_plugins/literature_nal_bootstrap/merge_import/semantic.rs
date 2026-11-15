use super::*;
use crate::runtime_plugins::literature_concepts::semantic::{SemanticResources, register};
use research_workbench_native_sdk::knowledge_snapshot::KnowledgeConcept;

impl Import<'_, '_> {
    pub(super) fn semantic(&self, actual: &KnowledgeConcept) -> Result<()> {
        register(
            self.tx,
            SemanticResources {
                legacy_render_concept: &self.rules.legacy_render_concept,
                representation_lookup: &self.storage.representation_lookup,
                semantic_old_jobs_cancel: &self.storage.semantic_old_jobs_cancel,
                semantic_old_records_tombstone: &self.storage.semantic_old_records_tombstone,
                representation_insert: &self.storage.representation_insert,
                semantic_space_policy: &self.rules.semantic_space_policy,
                embedding_space_insert: &self.storage.embedding_space_insert,
                generation_active: &self.storage.generation_active,
                generation_next: &self.storage.generation_next,
                semantic_generation_path: &self.rules.semantic_generation_path,
                generation_insert: &self.storage.generation_insert,
                embedding_job_lookup: &self.storage.embedding_job_lookup,
                semantic_record_row_id: &self.rules.semantic_record_row_id,
                embedding_record_insert: &self.storage.embedding_record_insert,
                semantic_job_idempotency: &self.rules.semantic_job_idempotency,
                embedding_job_insert: &self.storage.embedding_job_insert,
            },
            actual,
        )
        .map_err(|error| self.problem(error.phase, error.cause))
    }
}
