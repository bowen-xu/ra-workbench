use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{
    candidate_repository::get_candidate,
    candidate_schema::{CandidatePayload, decode_candidate_value},
    error::AiError,
    model::{AiAttentionFeedback, AiAttentionFeedbackDraft},
};

pub(crate) fn record_attention_feedback(
    policy: super::generation_policy::GenerationPolicySource,
    connection: &Connection,
    draft: AiAttentionFeedbackDraft,
) -> Result<AiAttentionFeedback, AiError> {
    validate_draft(&draft)?;
    if let Some(existing) = feedback_by_key(connection, &draft.idempotency_key)? {
        if existing.candidate_id != draft.candidate_id
            || existing.candidate_revision != draft.candidate_revision
            || existing.suggestion_id != draft.suggestion_id
            || existing.feedback_kind != draft.feedback_kind
            || existing.note != draft.note.trim()
        {
            return Err(AiError::InvalidInput(
                "注意力反馈幂等键已用于不同参数".to_owned(),
            ));
        }
        return Ok(existing);
    }
    let candidate = get_candidate(connection, &draft.candidate_id)?;
    if candidate.current_revision != draft.candidate_revision {
        return Err(AiError::InvalidInput(
            "候选版本已变化，请在当前版本重新反馈".to_owned(),
        ));
    }
    let revision = candidate
        .revisions
        .iter()
        .find(|revision| revision.revision == draft.candidate_revision)
        .ok_or_else(|| AiError::InvalidInput("找不到候选版本".to_owned()))?;
    let allowed = candidate
        .evidence_bundle
        .items
        .iter()
        .map(|item| item.citation_key.clone())
        .collect::<HashSet<_>>();
    let payload = decode_candidate_value(policy, revision.payload.clone(), &allowed)?;
    if !suggestion_ids(&payload).contains(draft.suggestion_id.as_str()) {
        return Err(AiError::InvalidInput(
            "注意力反馈目标不属于当前候选".to_owned(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "insert into ai_attention_feedback
         (id, candidate_id, candidate_revision, suggestion_id, feedback_kind, note,
          idempotency_key, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            id,
            draft.candidate_id,
            draft.candidate_revision,
            draft.suggestion_id,
            draft.feedback_kind,
            draft.note.trim(),
            draft.idempotency_key,
        ],
    )?;
    feedback_by_id(connection, &id)
}

pub(crate) fn list_attention_feedback(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<AiAttentionFeedback>, AiError> {
    let ids = connection
        .prepare(
            "select id from ai_attention_feedback
             where candidate_id = ?1 order by rowid",
        )?
        .query_map([candidate_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter()
        .map(|id| feedback_by_id(connection, id))
        .collect()
}

fn validate_draft(draft: &AiAttentionFeedbackDraft) -> Result<(), AiError> {
    if !matches!(
        draft.feedback_kind.as_str(),
        "useful" | "not_useful" | "dismissed"
    ) || draft.suggestion_id.trim().is_empty()
        || draft.suggestion_id.trim() != draft.suggestion_id
        || draft.suggestion_id.len() > 80
        || draft.note.chars().count() > 1_000
        || draft.idempotency_key.trim().is_empty()
        || draft.idempotency_key.trim() != draft.idempotency_key
        || draft.idempotency_key.len() > 200
    {
        return Err(AiError::InvalidInput("注意力反馈参数无效".to_owned()));
    }
    Ok(())
}

fn suggestion_ids(payload: &CandidatePayload) -> HashSet<&str> {
    match payload {
        CandidatePayload::PaperPortrait(portrait) => portrait
            .claims
            .iter()
            .map(|claim| claim.id.as_str())
            .collect(),
        CandidatePayload::Selection(selection) => selection
            .statements
            .iter()
            .map(|statement| statement.id.as_str())
            .collect(),
        CandidatePayload::PaperOverview(overview) => overview
            .fields
            .iter()
            .flat_map(|field| field.statements.iter())
            .map(|statement| statement.id.as_str())
            .collect(),
        CandidatePayload::PaperConceptMap(candidate) => candidate
            .concepts
            .iter()
            .map(|concept| concept.id.as_str())
            .chain(
                candidate
                    .relations
                    .iter()
                    .map(|relation| relation.id.as_str()),
            )
            .collect(),
        CandidatePayload::ConceptSuggestion(candidate) => candidate
            .suggestions
            .iter()
            .map(|suggestion| suggestion.id.as_str())
            .collect(),
        CandidatePayload::DefinitionSuggestion(candidate) => candidate
            .suggestions
            .iter()
            .map(|suggestion| suggestion.id.as_str())
            .collect(),
        CandidatePayload::RelationSuggestion(candidate) => candidate
            .suggestions
            .iter()
            .map(|suggestion| suggestion.id.as_str())
            .collect(),
        CandidatePayload::AttentionSuggestion(candidate) => candidate
            .suggestions
            .iter()
            .map(|suggestion| suggestion.id.as_str())
            .collect(),
        CandidatePayload::ReferenceRecord(_) => HashSet::new(),
        CandidatePayload::DerivedResearchView(candidate) => candidate
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect(),
    }
}

fn feedback_by_key(
    connection: &Connection,
    idempotency_key: &str,
) -> Result<Option<AiAttentionFeedback>, AiError> {
    let id = connection
        .query_row(
            "select id from ai_attention_feedback where idempotency_key = ?1",
            [idempotency_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    id.map(|id| feedback_by_id(connection, &id)).transpose()
}

fn feedback_by_id(connection: &Connection, id: &str) -> Result<AiAttentionFeedback, AiError> {
    connection
        .query_row(
            "select id, candidate_id, candidate_revision, suggestion_id, feedback_kind,
                    note, idempotency_key, created_at
             from ai_attention_feedback where id = ?1",
            [id],
            |row| {
                Ok(AiAttentionFeedback {
                    id: row.get(0)?,
                    candidate_id: row.get(1)?,
                    candidate_revision: row.get(2)?,
                    suggestion_id: row.get(3)?,
                    feedback_kind: row.get(4)?,
                    note: row.get(5)?,
                    idempotency_key: row.get(6)?,
                    created_at: row.get(7)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| AiError::InvalidInput("找不到注意力反馈".to_owned()))
}
