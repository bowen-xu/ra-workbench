//! Borrowed shared-candidate facts and review journal for internal native
//! composition. Neither handle exposes SQL, a transaction owner, or promotion.
use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

pub(crate) use super::candidate_repository::revise_candidate_with;
// Internal Core settlement adapters. Independent plugins receive finite owned DTOs.
pub(crate) use super::citation_assessment_repository::{
    current_payload_source, record_citation_assessment_with, validate_citation_readiness,
};
pub(crate) use super::generic_review::{
    GenericReviewAdapter, ReviewPlan, ReviewPromotion, decide_with,
};
pub(crate) use super::model::{
    AiCandidate, AiMatrixSynthesisContext, AiReviewDecisionDraft, AiSuggestionItemDecisionDraft,
    AiSuggestionResolutionDraft,
};
use super::{candidate_repository, error::AiError, matrix_synthesis_context_repository};

pub(crate) type GenericDecision = (String, i64, String, String);

pub(crate) struct ReviewFacts<'db> {
    db: &'db Connection,
}
impl<'db> ReviewFacts<'db> {
    /// Borrow the existing App guard or repository transaction.
    pub(crate) fn new(db: &'db Connection) -> Self {
        Self { db }
    }
    pub(crate) fn candidate(&self, id: &str) -> Result<AiCandidate, AiError> {
        candidate_repository::get_candidate(self.db, id)
    }
    pub(crate) fn require_completed_turn(&self, id: &str) -> Result<(), AiError> {
        candidate_repository::ensure_shared_turn_completed_for_candidate(self.db, id)
    }
    pub(crate) fn generic_decision(&self, key: &str) -> Result<Option<GenericDecision>, AiError> {
        Ok(self
            .db
            .query_row(
                "select candidate_id, candidate_revision, action, rationale
             from ai_review_decisions where idempotency_key = ?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?)
    }
    pub(crate) fn item_decision(&self, key: &str) -> Result<Option<(String, String)>, AiError> {
        Ok(self
            .db
            .query_row(
                "select candidate_id, request_hash from ai_suggestion_item_decisions
             where idempotency_key = ?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }
    pub(crate) fn item_is_terminal(
        &self,
        draft: &AiSuggestionItemDecisionDraft,
    ) -> Result<bool, AiError> {
        Ok(self.db.query_row(
            "select exists(select 1 from ai_suggestion_item_decisions
         where candidate_id = ?1 and candidate_revision = ?2 and suggestion_id = ?3
           and action in ('accept_unchanged', 'accept_after_edit', 'reject'))",
            params![
                draft.candidate_id,
                draft.candidate_revision,
                draft.suggestion_id
            ],
            |row| row.get(0),
        )?)
    }
    pub(crate) fn has_inspected_evidence(&self, id: &str, revision: i64) -> Result<bool, AiError> {
        Ok(self.db.query_row(
            "select exists(select 1 from ai_evidence_inspections
             where candidate_id = ?1 and candidate_revision = ?2)",
            params![id, revision],
            |row| row.get(0),
        )?)
    }
    pub(crate) fn inspected_keys(
        &self,
        draft: &AiSuggestionItemDecisionDraft,
    ) -> Result<HashSet<String>, AiError> {
        Ok(self
            .db
            .prepare(
                "select e.citation_key from ai_evidence_inspections i
         join ai_evidence_items e on e.id = i.evidence_item_id
         where i.candidate_id = ?1 and i.candidate_revision = ?2",
            )?
            .query_map(
                params![draft.candidate_id, draft.candidate_revision],
                |row| row.get(0),
            )?
            .collect::<Result<_, _>>()?)
    }
    pub(crate) fn item_counts(&self, id: &str, revision: i64) -> Result<(i64, i64), AiError> {
        Ok(self.db.query_row(
            "select count(distinct case when action in
                    ('accept_unchanged', 'accept_after_edit', 'reject') then suggestion_id end),
                count(distinct case when action in
                    ('accept_unchanged', 'accept_after_edit') then suggestion_id end)
         from ai_suggestion_item_decisions
         where candidate_id = ?1 and candidate_revision = ?2",
            params![id, revision],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }
    pub(crate) fn accepted_concept_targets(
        &self,
        candidate: &AiCandidate,
    ) -> Result<HashMap<String, String>, AiError> {
        Ok(self
            .db
            .prepare(
                "select suggestion_id, target_object_id from ai_suggestion_item_decisions
         where candidate_id = ?1 and candidate_revision = ?2
           and action in ('accept_unchanged', 'accept_after_edit')
           and target_object_kind = 'concept'",
            )?
            .query_map(params![candidate.id, candidate.current_revision], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?)
    }
    pub(crate) fn matrix_context(
        &self,
        run: &str,
    ) -> Result<Option<AiMatrixSynthesisContext>, AiError> {
        matrix_synthesis_context_repository::load_matrix_synthesis_context(self.db, run)
    }
}

pub(crate) struct ReviewTarget<'a> {
    pub kind: &'a str,
    pub id: &'a str,
    pub revision: i64,
}
pub(crate) struct ReviewJournal<'tx, 'db> {
    tx: &'tx Transaction<'db>,
}
impl<'tx, 'db> ReviewJournal<'tx, 'db> {
    /// Internal composition borrows the original transaction; no commit API.
    pub(crate) fn new(tx: &'tx Transaction<'db>) -> Self {
        Self { tx }
    }
    pub(crate) fn facts(&self) -> ReviewFacts<'_> {
        ReviewFacts::new(self.tx)
    }
    pub(crate) fn append_generic(
        &self,
        draft: &AiReviewDecisionDraft,
        target: Option<ReviewTarget<'_>>,
    ) -> Result<(), AiError> {
        self.tx.execute(
            "insert into ai_review_decisions
         (id, candidate_id, candidate_revision, action, rationale, idempotency_key,
          target_object_kind, target_object_id, target_object_revision, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                Uuid::new_v4().to_string(),
                draft.candidate_id,
                draft.candidate_revision,
                draft.action,
                draft.rationale.trim(),
                draft.idempotency_key,
                target.as_ref().map(|value| value.kind),
                target.as_ref().map(|value| value.id),
                target.as_ref().map(|value| value.revision)
            ],
        )?;
        Ok(())
    }
    pub(crate) fn append_item(
        &self,
        draft: &AiSuggestionItemDecisionDraft,
        request_hash: &str,
        resolution_kind: &str,
        target: Option<ReviewTarget<'_>>,
    ) -> Result<(), AiError> {
        self.tx.execute(
            "insert into ai_suggestion_item_decisions
         (id, candidate_id, candidate_revision, suggestion_id, action, resolution_kind,
          rationale, idempotency_key, request_hash, target_object_kind, target_object_id,
          target_object_revision, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                Uuid::new_v4().to_string(),
                draft.candidate_id,
                draft.candidate_revision,
                draft.suggestion_id,
                draft.action,
                resolution_kind,
                draft.rationale.trim(),
                draft.idempotency_key,
                request_hash,
                target.as_ref().map(|value| value.kind),
                target.as_ref().map(|value| value.id),
                target.as_ref().map(|value| value.revision)
            ],
        )?;
        Ok(())
    }
    pub(crate) fn set_review_state(&self, id: &str, state: &str) -> Result<(), AiError> {
        self.tx.execute(
            "update ai_candidates set review_state = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
            params![id, state],
        )?;
        Ok(())
    }
}
