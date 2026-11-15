use super::model::AiEvidenceItem;
use super::{
    error::AiError,
    generation_policy::GenerationPolicySource,
    task_evidence_support::{
        AiEvidenceBundle, AiEvidencePreview, TaskEvidenceKind, TaskEvidencePreview,
    },
};
use rusqlite::{Connection, OptionalExtension};
pub(crate) fn preview_selection_evidence(
    policy: GenerationPolicySource,
    connection: &Connection,
    anchor_id: &str,
) -> Result<Vec<AiEvidencePreview>, AiError> {
    (policy.evidence_preview)(connection, TaskEvidencePreview::Selection(anchor_id))
}
pub(crate) fn persist_selection_evidence(
    policy: GenerationPolicySource,
    connection: &mut Connection,
    run_id: &str,
) -> Result<AiEvidenceBundle, AiError> {
    (policy.evidence_persist)(connection, TaskEvidenceKind::Selection, run_id)
}
pub(crate) fn load_bundle(
    connection: &Connection,
    bundle_id: &str,
) -> Result<AiEvidenceBundle, AiError> {
    let mut bundle = connection
        .query_row(
            "select id, run_id, scope_hash, retrieval_config_version, validation_status, created_at
             from ai_evidence_bundles where id = ?1",
            [bundle_id],
            |row| {
                Ok(AiEvidenceBundle {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    scope_hash: row.get(2)?,
                    retrieval_config_version: row.get(3)?,
                    validation_status: row.get(4)?,
                    created_at: row.get(5)?,
                    items: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| AiError::InvalidInput("找不到 AI 证据包".to_owned()))?;
    let mut statement = connection.prepare(
        "select e.id, e.citation_key, e.ordinal, e.anchor_id, e.paper_id, e.paper_title,
                e.attachment_id, e.content_sha256, e.page_index, e.page_label, e.quote_exact,
                e.quote_prefix, e.quote_suffix, e.context_text, e.relation_kind,
                e.validation_status,
                case
                    when s.id is null or a.id is null then 'missing'
                    when s.item_id <> e.paper_id
                      or s.attachment_id <> e.attachment_id
                      or s.content_sha256 <> e.content_sha256
                      or a.content_sha256 <> e.content_sha256
                      or s.page_index <> e.page_index
                      or coalesce(s.page_label, '') <> coalesce(e.page_label, '')
                      or s.quote_exact <> e.quote_exact
                      or s.quote_prefix <> e.quote_prefix
                      or s.quote_suffix <> e.quote_suffix then 'stale'
                    when json_array_length(s.rects_json) = 0 then 'page_level'
                    else 'exact'
                end,
                e.created_at
         from ai_evidence_items e
         left join source_anchors s on s.id = e.anchor_id
         left join attachments a on a.id = s.attachment_id
         where e.bundle_id = ?1 order by e.ordinal",
    )?;
    bundle.items = statement
        .query_map([bundle_id], |row| {
            Ok(AiEvidenceItem {
                id: row.get(0)?,
                citation_key: row.get(1)?,
                ordinal: row.get(2)?,
                anchor_id: row.get(3)?,
                paper_id: row.get(4)?,
                paper_title: row.get(5)?,
                attachment_id: row.get(6)?,
                content_sha256: row.get(7)?,
                page_index: row.get(8)?,
                page_label: row.get(9)?,
                quote_exact: row.get(10)?,
                quote_prefix: row.get(11)?,
                quote_suffix: row.get(12)?,
                context_text: row.get(13)?,
                relation_kind: row.get(14)?,
                validation_status: row.get(15)?,
                current_anchor_status: row.get(16)?,
                created_at: row.get(17)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(bundle)
}
