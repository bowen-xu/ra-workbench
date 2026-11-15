use research_agent_core::{conversation::ConversationId, protocol::TurnOutcome};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    attention_feedback_repository::list_attention_feedback,
    candidate_summary_repository::list_summary_versions,
    citation_assessment_repository::list_citation_assessments,
    error::AiError,
    evidence_repository::load_bundle,
    model::{
        AiCandidate, AiCandidateRevision, AiCandidateRevisionDraft, AiEvidenceBundle,
        AiReviewDecision,
    },
    overview_confirmation_repository::list_overview_field_confirmations,
    suggestion_review_repository::list_suggestion_item_decisions,
};

#[path = "candidate_repository/generated.rs"]
mod generated;
#[cfg(test)]
pub(crate) use generated::save_prepared_candidate;
pub(crate) use generated::{
    GeneratedCandidateCommit, save_generated_candidate, save_prepared_candidate_guarded,
};
/// When a candidate has been produced by a migrated Agent turn, the durable
/// turn journal is the execution authority. Review actions may still operate
/// on legacy candidates without a mapping, but they must not promote a mapped
/// candidate before its shared turn is durably completed.
pub(crate) fn ensure_shared_turn_completed_for_candidate(
    connection: &Connection,
    candidate_id: &str,
) -> Result<(), AiError> {
    let table_exists = |name: &str| -> rusqlite::Result<bool> {
        connection.query_row(
            "select exists(select 1 from sqlite_master where type='table' and name=?1)",
            [name],
            |row| row.get(0),
        )
    };
    let mut mapped: Option<(String, String)> = None;
    if table_exists("agent_paper_analysis_submissions")? {
        mapped = connection
            .query_row(
                "select conversation_id, turn_id
                 from agent_paper_analysis_submissions
                 where candidate_id = ?1 limit 1",
                [candidate_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
    }
    if mapped.is_none()
        && table_exists("agent_legacy_ai_run_bindings")?
        && table_exists("agent_submissions")?
    {
        mapped = connection
            .query_row(
                "select binding.conversation_id, submission.turn_id
                 from agent_legacy_ai_run_bindings binding
                 join agent_submissions submission
                   on submission.conversation_id = binding.conversation_id
                  and submission.request_id = binding.request_id
                 where binding.candidate_id = ?1 limit 1",
                [candidate_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
    }
    let Some((conversation_id, turn_id)) = mapped else {
        return Ok(());
    };
    let conversation_id = ConversationId::from_uuid(
        Uuid::parse_str(&conversation_id).map_err(|_| AiError::TemporaryUnavailable)?,
    );
    let turn_id = Uuid::parse_str(&turn_id).map_err(|_| AiError::TemporaryUnavailable)?;
    match crate::agent::repository::projection::read_turn_outcome(
        connection,
        conversation_id,
        turn_id,
    )? {
        Some(TurnOutcome::Completed) => Ok(()),
        Some(_) | None => Err(AiError::TemporaryUnavailable),
    }
}

pub(crate) fn revise_candidate(
    policy: super::generation_policy::GenerationPolicySource,
    connection: &mut Connection,
    draft: AiCandidateRevisionDraft,
) -> Result<AiCandidate, AiError> {
    revise_candidate_with(connection, draft, false, |db, candidate, value| {
        (policy.revised)(
            db,
            &candidate.run_id,
            &candidate.id,
            &candidate.candidate_kind,
            &candidate.evidence_bundle,
            value,
        )
    })
}

/// The original revision transaction pauses only at its domain validation boundary.
pub(crate) fn revise_candidate_with<E>(
    connection: &mut Connection,
    draft: AiCandidateRevisionDraft,
    explicit_settlement: bool,
    prepare: impl FnOnce(
        &Connection,
        &AiCandidate,
        serde_json::Value,
    ) -> Result<super::generation_policy::PreparedCandidate, E>,
) -> Result<AiCandidate, E>
where
    E: From<AiError> + From<rusqlite::Error>,
{
    if draft.change_reason.trim().chars().count() > 500 {
        return Err(AiError::InvalidInput("候选修订说明过长".to_owned()).into());
    }
    let candidate = get_candidate(connection, &draft.candidate_id)?;
    if is_terminal(&candidate.review_state) {
        return Err(AiError::InvalidInput("已结束审查的候选不能再修改".to_owned()).into());
    }
    if candidate.suggestion_item_decisions.iter().any(|decision| {
        matches!(
            decision.action.as_str(),
            "accept_unchanged" | "accept_after_edit" | "reject"
        )
    }) {
        return Err(
            AiError::InvalidInput("候选已开始逐条终态审查，不能再改写整批建议".to_owned()).into(),
        );
    }
    if candidate.current_revision != draft.expected_revision {
        return Err(
            AiError::InvalidInput("候选已产生新版本，请重新载入后再修改".to_owned()).into(),
        );
    }
    let prepared = prepare(connection, &candidate, draft.payload)?;
    let payload_json = prepared.canonical_json;
    let payload_hash = hash_text(&payload_json);
    let next_revision = candidate.current_revision + 1;
    let transaction = connection.transaction()?;
    let result = (|| -> Result<(), rusqlite::Error> {
        transaction.execute(
            "insert into ai_candidate_revisions
         (id, candidate_id, revision, payload_json, payload_hash, editor_kind,
          source_run_id, parent_revision, change_reason, created_at)
         values (?1, ?2, ?3, ?4, ?5, 'human', null, ?6, ?7,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                Uuid::new_v4().to_string(),
                draft.candidate_id,
                next_revision,
                payload_json,
                payload_hash,
                candidate.current_revision,
                draft.change_reason.trim(),
            ],
        )?;
        transaction.execute(
            "update ai_candidates set current_revision = ?2, review_state = 'edited',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
            params![draft.candidate_id, next_revision],
        )?;
        Ok(())
    })()
    .map_err(|error| {
        super::structured_resource_validation::Flow::<AiError, std::convert::Infallible>::original(
            error.into(),
        )
    });
    let settled = crate::hbs_storage::boundary::settle_with(
        transaction,
        result,
        "write",
        explicit_settlement,
        true,
        AiError::from,
        |_| AiError::TemporaryUnavailable,
    );
    match crate::hbs_storage::boundary::confirm_with(
        connection,
        settled,
        explicit_settlement,
        |_| AiError::TemporaryUnavailable,
    ) {
        Ok(()) => get_candidate(connection, &candidate.id).map_err(Into::into),
        Err(crate::hbs_storage::boundary::BoundaryError::Failure(failure))
            if explicit_settlement && failure.needs_recovery() =>
        {
            Err(AiError::GenerationStorageUnknown(Box::new(
                super::generation_storage_failure::GenerationStorageUnknown::from_failure(
                    &candidate.run_id,
                    "candidate_revision",
                    Some(candidate.id.clone()),
                    failure,
                ),
            ))
            .into())
        }
        Err(error) => Err(error.into_static().into()),
    }
}

pub(crate) fn list_candidates(
    connection: &Connection,
    project_id: Option<&str>,
    paper_id: Option<&str>,
) -> Result<Vec<AiCandidate>, AiError> {
    // Performance invariant: scope paper detail IDs in SQL before get_candidate
    // assembles evidence and full revision history. Loading every paper and then
    // filtering in the frontend caused a visible stall on each dev:desktop selection.
    // None is reserved for intentional global/project history callers. Regression:
    // tests::paper_scope_filters_before_loading_other_papers_history.
    let mut statement = connection.prepare(
        "select c.id
         from ai_candidates c
         join ai_evidence_items e on e.bundle_id = c.evidence_bundle_id and e.ordinal = 0
         where (?1 is null or exists(
             select 1 from project_items p where p.project_id = ?1 and p.item_id = e.paper_id
         ))
         and (?2 is null or (e.paper_id = ?2 and not exists(
             select 1 from ai_evidence_items other
             where other.bundle_id = c.evidence_bundle_id and other.paper_id != ?2
         )))
         order by c.updated_at desc, c.id desc",
    )?;
    let ids = statement
        .query_map(params![project_id, paper_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut candidates = Vec::new();
    for id in ids {
        match get_candidate(connection, &id) {
            Ok(candidate) => candidates.push(candidate),
            Err(AiError::InvalidInput(_)) => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(candidates)
}

pub(crate) fn get_candidate(
    connection: &Connection,
    candidate_id: &str,
) -> Result<AiCandidate, AiError> {
    let mut candidate = connection
        .query_row(
            "select c.id, c.run_id, r.provider_id, r.model_id, c.candidate_kind,
                    c.schema_version, c.current_revision, c.review_state,
                    c.validation_status, c.original_payload_hash, c.created_at,
                    c.updated_at, c.evidence_bundle_id
             from ai_candidates c join ai_runs r on r.id = c.run_id where c.id = ?1",
            [candidate_id],
            |row| {
                Ok((
                    AiCandidate {
                        id: row.get(0)?,
                        run_id: row.get(1)?,
                        provider_id: row.get(2)?,
                        model_id: row.get(3)?,
                        candidate_kind: row.get(4)?,
                        schema_version: row.get(5)?,
                        current_revision: row.get(6)?,
                        review_state: row.get(7)?,
                        validation_status: row.get(8)?,
                        original_payload_hash: row.get(9)?,
                        created_at: row.get(10)?,
                        updated_at: row.get(11)?,
                        evidence_bundle: AiEvidenceBundle {
                            id: String::new(),
                            run_id: String::new(),
                            scope_hash: String::new(),
                            retrieval_config_version: String::new(),
                            validation_status: String::new(),
                            created_at: String::new(),
                            items: Vec::new(),
                        },
                        revisions: Vec::new(),
                        decisions: Vec::new(),
                        attention_feedback: Vec::new(),
                        suggestion_item_decisions: Vec::new(),
                        citation_assessments: Vec::new(),
                        inspected_evidence_item_ids: Vec::new(),
                        overview_field_confirmations: Vec::new(),
                        summary_versions: Vec::new(),
                    },
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| AiError::InvalidInput("找不到 AI 候选".to_owned()))?;
    candidate.0.evidence_bundle = load_bundle(connection, &candidate.1)?;
    candidate.0.revisions = list_revisions(connection, candidate_id)?;
    candidate.0.decisions = list_decisions(connection, candidate_id)?;
    candidate.0.attention_feedback = list_attention_feedback(connection, candidate_id)?;
    candidate.0.suggestion_item_decisions =
        list_suggestion_item_decisions(connection, candidate_id)?;
    candidate.0.citation_assessments = list_citation_assessments(connection, candidate_id)?;
    candidate.0.inspected_evidence_item_ids =
        list_inspected_evidence_item_ids(connection, candidate_id, candidate.0.current_revision)?;
    candidate.0.overview_field_confirmations =
        list_overview_field_confirmations(connection, candidate_id)?;
    candidate.0.summary_versions = list_summary_versions(connection, candidate_id)?;
    if candidate
        .0
        .overview_field_confirmations
        .iter()
        .any(|confirmation| confirmation.active)
        && !matches!(
            candidate.0.review_state.as_str(),
            "accepted_unchanged" | "accepted_after_edit" | "rejected"
        )
    {
        candidate.0.review_state = "partially_accepted".to_owned();
    }
    Ok(candidate.0)
}

fn list_inspected_evidence_item_ids(
    connection: &Connection,
    candidate_id: &str,
    candidate_revision: i64,
) -> Result<Vec<String>, AiError> {
    let mut statement = connection.prepare(
        "select evidence_item_id from ai_evidence_inspections
         where candidate_id = ?1 and candidate_revision = ?2
         order by viewed_at, evidence_item_id",
    )?;
    Ok(statement
        .query_map(params![candidate_id, candidate_revision], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn list_revisions(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<AiCandidateRevision>, AiError> {
    let mut statement = connection.prepare(
        "select id, revision, payload_json, payload_hash, editor_kind, source_run_id,
                parent_revision, change_reason, created_at
         from ai_candidate_revisions where candidate_id = ?1 order by revision",
    )?;
    statement
        .query_map([candidate_id], |row| {
            let payload_json = row.get::<_, String>(2)?;
            let payload = serde_json::from_str(&payload_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    payload_json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(AiCandidateRevision {
                id: row.get(0)?,
                revision: row.get(1)?,
                payload,
                payload_hash: row.get(3)?,
                editor_kind: row.get(4)?,
                source_run_id: row.get(5)?,
                parent_revision: row.get(6)?,
                change_reason: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn list_decisions(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<AiReviewDecision>, AiError> {
    let mut statement = connection.prepare(
        "select id, candidate_id, candidate_revision, action, rationale, idempotency_key,
                target_object_kind, target_object_id, target_object_revision, created_at
         from ai_review_decisions where candidate_id = ?1 order by created_at, id",
    )?;
    Ok(statement
        .query_map([candidate_id], |row| {
            Ok(AiReviewDecision {
                id: row.get(0)?,
                candidate_id: row.get(1)?,
                candidate_revision: row.get(2)?,
                action: row.get(3)?,
                rationale: row.get(4)?,
                idempotency_key: row.get(5)?,
                target_object_kind: row.get(6)?,
                target_object_id: row.get(7)?,
                target_object_revision: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn hash_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn is_terminal(review_state: &str) -> bool {
    matches!(
        review_state,
        "accepted_unchanged" | "accepted_after_edit" | "rejected"
    )
}

#[cfg(test)]
#[path = "candidate_repository_tests.rs"]
mod tests;
