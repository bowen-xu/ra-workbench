use super::{
    error::AiError,
    generation_policy::GenerationPolicySource,
    task_evidence_support::{AiEvidenceBundle, AiEvidencePreview, TaskEvidencePreview},
};
use rusqlite::Connection;
pub(crate) fn preview_overview_evidence(
    policy: GenerationPolicySource,
    connection: &Connection,
    attachment_id: &str,
) -> Result<Vec<AiEvidencePreview>, AiError> {
    (policy.evidence_preview)(connection, TaskEvidencePreview::Pages(attachment_id))
}

pub(crate) fn preview_frozen_overview_evidence(
    bundle: &AiEvidenceBundle,
) -> Vec<AiEvidencePreview> {
    bundle
        .items
        .iter()
        .map(|item| AiEvidencePreview {
            citation_key: item.citation_key.clone(),
            anchor_id: item.anchor_id.clone(),
            paper_title: item.paper_title.clone(),
            attachment_id: item.attachment_id.clone(),
            content_sha256: item.content_sha256.clone(),
            page_index: item.page_index,
            page_label: item.page_label.clone(),
            quote_exact: item.quote_exact.clone(),
            context_text: item.context_text.clone(),
            relation_kind: item.relation_kind.clone(),
        })
        .collect()
}
