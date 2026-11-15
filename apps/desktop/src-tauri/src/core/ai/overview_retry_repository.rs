use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    candidate_repository::get_candidate,
    candidate_schema::{CandidatePayload, PaperOverviewPayload, decode_candidate_value},
    error::AiError,
    model::{
        AiCandidate, AiEvidenceBundle, AiEvidencePreview, AiOverviewRetryContext, AiScopeItem,
        AiTaskSpecDraft,
    },
    overview_evidence_repository::preview_frozen_overview_evidence,
};

pub(crate) fn validate_retry_shape(draft: &AiTaskSpecDraft) -> Result<(), AiError> {
    let has_candidate = draft.base_candidate_id.is_some();
    let has_revision = draft.base_candidate_revision.is_some();
    let has_fields = !draft.overview_fields.is_empty();
    if has_candidate != has_revision || has_candidate != has_fields {
        return Err(AiError::InvalidInput(
            "概要字段重试必须同时指定候选、基础版本和目标字段".to_owned(),
        ));
    }
    if !has_candidate {
        return Ok(());
    }
    if draft.task_kind != "paper_overview" {
        return Err(AiError::InvalidInput(
            "只有论文概要任务支持字段级重试".to_owned(),
        ));
    }
    if draft.overview_profile_selection_revision.is_some() {
        return Err(AiError::InvalidInput(
            "概要字段重试继承原候选画像，不能另行指定版本".to_owned(),
        ));
    }
    let mut seen = HashSet::new();
    if draft
        .overview_fields
        .iter()
        .any(|field| !super::paper_portrait::identity(field) || !seen.insert(field.as_str()))
    {
        return Err(AiError::InvalidInput(
            "概要字段重试包含未知或重复字段".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_retry_candidate(
    policy: super::generation_policy::GenerationPolicySource,
    connection: &Connection,
    draft: &AiTaskSpecDraft,
    scope_item: &AiScopeItem,
) -> Result<Option<(AiOverviewRetryContext, AiEvidenceBundle)>, AiError> {
    let Some(candidate_id) = draft.base_candidate_id.as_deref() else {
        return Ok(None);
    };
    let base_revision = draft
        .base_candidate_revision
        .ok_or_else(|| AiError::InvalidInput("概要字段重试缺少基础版本".to_owned()))?;
    let candidate = get_candidate(connection, candidate_id)?;
    validate_retry_base(&candidate, base_revision, scope_item)?;
    validate_retry_targets_unconfirmed(&candidate, &draft.overview_fields)?;
    let payload = current_overview_payload(policy, &candidate)?;
    for target in &draft.overview_fields {
        let field = payload
            .fields
            .iter()
            .find(|field| field.field == *target)
            .ok_or_else(|| AiError::InvalidInput("概要候选缺少目标字段".to_owned()))?;
        if field.status == "filled" {
            return Err(AiError::InvalidInput(
                "只能重试部分完成或未找到来源的概要字段".to_owned(),
            ));
        }
    }
    let target_fields = payload
        .fields
        .iter()
        .filter(|field| draft.overview_fields.contains(&field.field))
        .map(|field| field.field.clone())
        .collect();
    Ok(Some((
        AiOverviewRetryContext {
            candidate_id: candidate.id,
            base_revision,
            target_fields,
        },
        candidate.evidence_bundle,
    )))
}

pub(crate) fn retry_evidence_preview(bundle: &AiEvidenceBundle) -> Vec<AiEvidencePreview> {
    preview_frozen_overview_evidence(bundle)
}

pub(crate) fn insert_retry_context(
    transaction: &Transaction<'_>,
    run_id: &str,
    context: &AiOverviewRetryContext,
) -> Result<(), AiError> {
    let target_fields_json = serde_json::to_string(&context.target_fields)
        .map_err(|_| AiError::InvalidInput("无法规范化概要重试字段".to_owned()))?;
    transaction.execute(
        "insert into ai_overview_retry_runs
         (run_id, candidate_id, base_revision, target_fields_json, created_at)
         values (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            run_id,
            context.candidate_id,
            context.base_revision,
            target_fields_json
        ],
    )?;
    Ok(())
}

pub(crate) fn load_retry_context(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<AiOverviewRetryContext>, AiError> {
    let stored = connection
        .query_row(
            "select candidate_id, base_revision, target_fields_json
             from ai_overview_retry_runs where run_id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    stored
        .map(|(candidate_id, base_revision, fields_json)| {
            let target_fields = serde_json::from_str::<Vec<String>>(&fields_json)
                .map_err(|_| AiError::InvalidInput("概要重试记录已损坏".to_owned()))?;
            validate_stored_fields(&target_fields)?;
            Ok(AiOverviewRetryContext {
                candidate_id,
                base_revision,
                target_fields,
            })
        })
        .transpose()
}

fn validate_retry_base(
    candidate: &AiCandidate,
    base_revision: i64,
    scope_item: &AiScopeItem,
) -> Result<(), AiError> {
    let attachment_matches = candidate.evidence_bundle.items.iter().all(|item| {
        item.attachment_id == scope_item.object_id
            && scope_item.content_sha256.as_ref() == Some(&item.content_sha256)
    });
    if candidate.candidate_kind != "paper_overview_unit_v1"
        || candidate.current_revision != base_revision
        || is_terminal(&candidate.review_state)
        || !attachment_matches
    {
        return Err(AiError::InvalidInput(
            "概要候选、版本或 PDF 范围已变化，请重新选择字段".to_owned(),
        ));
    }
    Ok(())
}

fn validate_retry_targets_unconfirmed(
    candidate: &AiCandidate,
    target_fields: &[String],
) -> Result<(), AiError> {
    if candidate
        .overview_field_confirmations
        .iter()
        .any(|confirmation| {
            confirmation.active
                && target_fields
                    .iter()
                    .any(|target| target == &confirmation.field_kind)
        })
    {
        return Err(AiError::InvalidInput(
            "已确认的概要字段不能再次由 AI 重试覆盖；请先撤销确认".to_owned(),
        ));
    }
    Ok(())
}

fn current_overview_payload(
    policy: super::generation_policy::GenerationPolicySource,
    candidate: &AiCandidate,
) -> Result<PaperOverviewPayload, AiError> {
    let revision = candidate
        .revisions
        .iter()
        .find(|revision| revision.revision == candidate.current_revision)
        .ok_or_else(|| AiError::InvalidInput("概要候选缺少当前版本".to_owned()))?;
    let allowed = evidence_keys(&candidate.evidence_bundle);
    overview_payload(decode_candidate_value(
        policy,
        revision.payload.clone(),
        &allowed,
    )?)
}

fn overview_payload(payload: CandidatePayload) -> Result<PaperOverviewPayload, AiError> {
    match payload {
        CandidatePayload::PaperOverview(overview) => Ok(overview),
        CandidatePayload::PaperPortrait(_)
        | CandidatePayload::Selection(_)
        | CandidatePayload::PaperConceptMap(_)
        | CandidatePayload::ConceptSuggestion(_)
        | CandidatePayload::DefinitionSuggestion(_)
        | CandidatePayload::RelationSuggestion(_)
        | CandidatePayload::AttentionSuggestion(_)
        | CandidatePayload::ReferenceRecord(_)
        | CandidatePayload::DerivedResearchView(_) => Err(AiError::InvalidInput(
            "字段级重试只能用于论文概要候选".to_owned(),
        )),
    }
}

fn validate_stored_fields(fields: &[String]) -> Result<(), AiError> {
    let mut seen = HashSet::new();
    if fields.is_empty()
        || fields
            .iter()
            .any(|field| !super::paper_portrait::identity(field) || !seen.insert(field))
    {
        return Err(AiError::InvalidInput("概要重试字段记录无效".to_owned()));
    }
    Ok(())
}

fn evidence_keys(bundle: &AiEvidenceBundle) -> HashSet<String> {
    bundle
        .items
        .iter()
        .map(|item| item.citation_key.clone())
        .collect()
}

fn is_terminal(review_state: &str) -> bool {
    matches!(
        review_state,
        "accepted_unchanged" | "accepted_after_edit" | "rejected"
    )
}
