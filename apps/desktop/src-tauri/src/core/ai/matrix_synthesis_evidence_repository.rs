use super::{
    error::AiError,
    generation_policy::GenerationPolicySource,
    task_evidence_support::{AiEvidencePreview, TaskEvidencePreview},
};
use rusqlite::Connection;
pub(crate) fn preview_matrix_synthesis_evidence(
    policy: GenerationPolicySource,
    connection: &Connection,
    anchor_ids: &[String],
) -> Result<Vec<AiEvidencePreview>, AiError> {
    (policy.evidence_preview)(connection, TaskEvidencePreview::Matrix(anchor_ids))
}
