use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    candidate_repository::get_candidate,
    candidate_schema::{CandidatePayload, decode_candidate_value},
    error::AiError,
    model::{AiCandidate, AiCitationAssessment, AiCitationAssessmentDraft},
};

const POSITIVE_RELATIONS: [&str; 2] = ["supports", "partially_supports"];
const NEGATIVE_RELATIONS: [&str; 2] = ["opposes", "unsupported"];

pub(crate) fn record_citation_assessment(
    policy: super::generation_policy::GenerationPolicySource,
    connection: &Connection,
    draft: AiCitationAssessmentDraft,
) -> Result<AiCandidate, AiError> {
    record_citation_assessment_with(connection, draft, |candidate| {
        current_payload(policy, candidate)
    })
}

/// Decode at the original consumption point on the caller's connection.
pub(crate) fn record_citation_assessment_with<E>(
    connection: &Connection,
    mut draft: AiCitationAssessmentDraft,
    decode: impl FnOnce(&AiCandidate) -> Result<CandidatePayload, E>,
) -> Result<AiCandidate, E>
where
    E: From<AiError> + From<rusqlite::Error>,
{
    normalize_draft(&mut draft)?;
    let request_hash = request_hash(&draft)?;
    if let Some((candidate_id, existing_hash)) = connection
        .query_row(
            "select candidate_id, request_hash from ai_citation_assessments
             where idempotency_key = ?1",
            [&draft.idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    {
        if candidate_id != draft.candidate_id || existing_hash != request_hash {
            return Err(AiError::InvalidInput("引用判断幂等键已被不同参数使用".to_owned()).into());
        }
        return get_candidate(connection, &candidate_id).map_err(Into::into);
    }

    let candidate = get_candidate(connection, &draft.candidate_id)?;
    if candidate.current_revision != draft.candidate_revision {
        return Err(AiError::InvalidInput("候选版本已变化，请重新判断当前引用".to_owned()).into());
    }
    if matches!(
        candidate.review_state.as_str(),
        "accepted_unchanged" | "accepted_after_edit" | "rejected"
    ) {
        return Err(AiError::InvalidInput("该候选审查已经结束".to_owned()).into());
    }
    let payload = decode(&candidate)?;
    let evidence = candidate
        .evidence_bundle
        .items
        .iter()
        .find(|item| item.id == draft.evidence_item_id)
        .ok_or_else(|| AiError::InvalidInput("找不到待判断的引用".to_owned()))?;
    let is_referenced = payload.citation_references().iter().any(|reference| {
        reference.statement_id == draft.statement_id
            && reference
                .evidence_item_ids
                .iter()
                .any(|citation_key| citation_key == &evidence.citation_key)
    });
    if !is_referenced {
        return Err(AiError::InvalidInput("该引用不属于指定的候选陈述".to_owned()).into());
    }
    let current_revision = connection.query_row(
        "select coalesce(max(assessment_revision), 0) from ai_citation_assessments
         where candidate_id = ?1 and candidate_revision = ?2
           and statement_id = ?3 and evidence_item_id = ?4",
        params![
            draft.candidate_id,
            draft.candidate_revision,
            draft.statement_id,
            draft.evidence_item_id
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if current_revision != draft.expected_assessment_revision {
        return Err(
            AiError::InvalidInput("引用判断已产生新版本，请重新载入后再修改".to_owned()).into(),
        );
    }
    connection.execute(
        "insert into ai_citation_assessments
         (id, candidate_id, candidate_revision, statement_id, evidence_item_id,
          assessment_revision, semantic_relation, rationale, idempotency_key,
          request_hash, assessed_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            Uuid::new_v4().to_string(),
            draft.candidate_id,
            draft.candidate_revision,
            draft.statement_id,
            draft.evidence_item_id,
            current_revision + 1,
            draft.semantic_relation,
            draft.rationale,
            draft.idempotency_key,
            request_hash,
        ],
    )?;
    get_candidate(connection, &draft.candidate_id).map_err(Into::into)
}

pub(crate) fn list_citation_assessments(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<AiCitationAssessment>, AiError> {
    let mut statement = connection.prepare(
        "select id, candidate_id, candidate_revision, statement_id, evidence_item_id,
                assessment_revision, semantic_relation, rationale, idempotency_key,
                assessed_at
         from ai_citation_assessments where candidate_id = ?1
         order by candidate_revision, statement_id, evidence_item_id, assessment_revision",
    )?;
    Ok(statement
        .query_map([candidate_id], |row| {
            Ok(AiCitationAssessment {
                id: row.get(0)?,
                candidate_id: row.get(1)?,
                candidate_revision: row.get(2)?,
                statement_id: row.get(3)?,
                evidence_item_id: row.get(4)?,
                assessment_revision: row.get(5)?,
                semantic_relation: row.get(6)?,
                rationale: row.get(7)?,
                idempotency_key: row.get(8)?,
                assessed_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn validate_citation_readiness(
    candidate: &AiCandidate,
    payload: &CandidatePayload,
    only_statement_id: Option<&str>,
) -> Result<(), AiError> {
    for reference in payload
        .citation_references()
        .into_iter()
        .filter(|reference| {
            !reference.evidence_item_ids.is_empty()
                && only_statement_id
                    .is_none_or(|statement_id| reference.statement_id == statement_id)
        })
    {
        let mut has_positive = false;
        for citation_key in reference.evidence_item_ids {
            let evidence = candidate
                .evidence_bundle
                .items
                .iter()
                .find(|item| item.citation_key == *citation_key)
                .ok_or_else(|| AiError::InvalidInput("候选引用已不可解析".to_owned()))?;
            if matches!(evidence.current_anchor_status.as_str(), "stale" | "missing") {
                return Err(AiError::InvalidInput(
                    "候选包含已失效或缺失的来源锚点".to_owned(),
                ));
            }
            if let Some(latest) =
                latest_assessment(candidate, &reference.statement_id, &evidence.id)
            {
                if NEGATIVE_RELATIONS.contains(&latest.semantic_relation.as_str()) {
                    return Err(AiError::InvalidInput(
                        "候选包含被人工判断为反对或不支持的引用".to_owned(),
                    ));
                }
                has_positive |= POSITIVE_RELATIONS.contains(&latest.semantic_relation.as_str());
            }
        }
        if !has_positive {
            return Err(AiError::InvalidInput(
                "接受前，每条候选陈述至少需要一条人工确认支持的引用".to_owned(),
            ));
        }
    }
    Ok(())
}

fn latest_assessment<'a>(
    candidate: &'a AiCandidate,
    statement_id: &str,
    evidence_item_id: &str,
) -> Option<&'a AiCitationAssessment> {
    candidate
        .citation_assessments
        .iter()
        .filter(|assessment| {
            assessment.candidate_revision == candidate.current_revision
                && assessment.statement_id == statement_id
                && assessment.evidence_item_id == evidence_item_id
        })
        .max_by_key(|assessment| assessment.assessment_revision)
}

fn current_payload(
    policy: super::generation_policy::GenerationPolicySource,
    candidate: &AiCandidate,
) -> Result<CandidatePayload, AiError> {
    let (value, keys) = current_payload_source(candidate)?;
    decode_candidate_value(policy, value, &keys.into_iter().collect())
}

pub(crate) fn current_payload_source(
    candidate: &AiCandidate,
) -> Result<(serde_json::Value, Vec<String>), AiError> {
    let revision = candidate
        .revisions
        .iter()
        .find(|value| value.revision == candidate.current_revision)
        .ok_or_else(|| AiError::InvalidInput("找不到候选当前版本".to_owned()))?;
    let allowed = candidate
        .evidence_bundle
        .items
        .iter()
        .map(|item| item.citation_key.clone())
        .collect::<Vec<_>>();
    Ok((revision.payload.clone(), allowed))
}

fn normalize_draft(draft: &mut AiCitationAssessmentDraft) -> Result<(), AiError> {
    draft.statement_id = draft.statement_id.trim().to_owned();
    draft.rationale = draft.rationale.trim().to_owned();
    draft.idempotency_key = draft.idempotency_key.trim().to_owned();
    if draft.candidate_revision < 1
        || draft.expected_assessment_revision < 0
        || draft.statement_id.is_empty()
        || draft.statement_id.len() > 160
        || draft.evidence_item_id.is_empty()
        || draft.rationale.chars().count() > 2_000
        || draft.idempotency_key.is_empty()
        || draft.idempotency_key.len() > 200
        || !matches!(
            draft.semantic_relation.as_str(),
            "supports"
                | "partially_supports"
                | "context_only"
                | "opposes"
                | "unsupported"
                | "uncertain"
        )
    {
        return Err(AiError::InvalidInput("引用判断参数无效".to_owned()));
    }
    Ok(())
}

fn request_hash(draft: &AiCitationAssessmentDraft) -> Result<String, AiError> {
    let bytes = serde_json::to_vec(draft)
        .map_err(|_| AiError::InvalidInput("无法规范化引用判断".to_owned()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
