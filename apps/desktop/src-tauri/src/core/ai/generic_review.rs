//! Original generic review reads and atomic journal, with domain decisions supplied at narrow seams.
use super::{
    candidate_contracts::CandidatePayload,
    error::AiError,
    review_support::{
        AiCandidate, AiReviewDecisionDraft, ReviewFacts, ReviewJournal, ReviewTarget,
        validate_citation_readiness,
    },
    structured_resource_validation::Flow,
};
use crate::hbs_storage::boundary::{self, BoundaryError};
use research_workbench_native_sdk::candidate_review::wire::{
    GenericReviewSemanticsV1, ReviewDecisionV1, ReviewInputV1,
};
use rusqlite::{Connection, Transaction};
use std::collections::HashSet;

pub(crate) struct ReviewPlan<'a> {
    pub(crate) draft: &'a AiReviewDecisionDraft,
    pub(crate) candidate: &'a AiCandidate,
    pub(crate) payload: &'a CandidatePayload,
    pub(crate) provenance: Option<&'a str>,
    pub(crate) paper_id: &'a str,
    pub(crate) anchor_ids: &'a [String],
}
pub(crate) type ReviewPromotion<P> = Result<Option<(String, String, i64)>, Flow<AiError, P>>;
pub(crate) trait GenericReviewAdapter {
    type Pause;
    fn review(
        &mut self,
        input: ReviewInputV1,
    ) -> Result<ReviewDecisionV1, Flow<AiError, Self::Pause>>;
    fn decode(
        &mut self,
        candidate: &AiCandidate,
        value: serde_json::Value,
    ) -> Result<CandidatePayload, Flow<AiError, Self::Pause>>;
    fn project_contains_paper(
        &mut self,
        db: &Connection,
        project: &str,
        paper: &str,
    ) -> Result<bool, Flow<AiError, Self::Pause>>;
    fn promote(
        &mut self,
        tx: &Transaction<'_>,
        plan: &ReviewPlan<'_>,
    ) -> ReviewPromotion<Self::Pause>;
}
fn validated<P>(decision: ReviewDecisionV1) -> Result<(), Flow<AiError, P>> {
    match decision {
        ReviewDecisionV1::Validated { rejection: None } => Ok(()),
        ReviewDecisionV1::Validated {
            rejection: Some(error),
        } => Err(Flow::original(error.into())),
        _ => Err(Flow::original(AiError::TemporaryUnavailable)),
    }
}

pub(crate) fn decide_with<A: GenericReviewAdapter>(
    db: &mut Connection,
    draft: AiReviewDecisionDraft,
    explicit_settlement: bool,
    adapter: &mut A,
) -> Result<AiCandidate, Flow<AiError, A::Pause>> {
    validated(adapter.review(ReviewInputV1::ValidateDraft {
        draft: draft.clone(),
    })?)?;
    if let Some(existing) = ReviewFacts::new(db)
        .generic_decision(&draft.idempotency_key)
        .map_err(Flow::original)?
    {
        validated(adapter.review(ReviewInputV1::Replay {
            draft: draft.clone(),
            existing,
        })?)?;
        return ReviewFacts::new(db)
            .candidate(&draft.candidate_id)
            .map_err(Flow::original);
    }
    let candidate = ReviewFacts::new(db)
        .candidate(&draft.candidate_id)
        .map_err(Flow::original)?;
    ReviewFacts::new(db)
        .require_completed_turn(&candidate.id)
        .map_err(Flow::original)?;
    let decision = adapter.review(ReviewInputV1::Candidate {
        draft: draft.clone(),
        observation: candidate.review_observation(draft.candidate_revision),
    })?;
    let semantics: GenericReviewSemanticsV1 = match decision {
        ReviewDecisionV1::Candidate { result } => {
            result.map_err(|error| Flow::original(error.into()))?
        }
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    if semantics.requires_inspection {
        let inspected = ReviewFacts::new(db)
            .has_inspected_evidence(&draft.candidate_id, draft.candidate_revision)
            .map_err(Flow::original)?;
        validated(adapter.review(ReviewInputV1::Inspection { inspected })?)?;
    }
    let revision = candidate
        .revisions
        .iter()
        .find(|revision| revision.revision == draft.candidate_revision)
        .ok_or_else(|| Flow::original(AiError::InvalidInput("找不到待审查的候选版本".into())))?;
    let payload = adapter.decode(&candidate, revision.payload.clone())?;
    if semantics.requires_citations {
        validate_citation_readiness(&candidate, &payload, None).map_err(Flow::original)?;
    }
    let evidence = candidate
        .evidence_bundle
        .items
        .first()
        .ok_or_else(|| Flow::original(AiError::InvalidInput("候选缺少可验证证据".into())))?;
    if let Some(project) = draft.project_id.as_deref() {
        let contains = adapter.project_contains_paper(db, project, &evidence.paper_id)?;
        validated(adapter.review(ReviewInputV1::ProjectMembership { contains })?)?;
    }
    let anchors = candidate
        .evidence_bundle
        .items
        .iter()
        .map(|item| item.anchor_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let plan = ReviewPlan {
        draft: &draft,
        candidate: &candidate,
        payload: &payload,
        provenance: semantics.provenance.as_deref(),
        paper_id: &evidence.paper_id,
        anchor_ids: &anchors,
    };
    let tx = db
        .transaction()
        .map_err(|error| Flow::original(error.into()))?;
    let result = (|| {
        let target = if plan.provenance.is_some() {
            adapter.promote(&tx, &plan)?
        } else {
            None
        };
        let journal = ReviewJournal::new(&tx);
        journal
            .append_generic(
                &draft,
                target.as_ref().map(|target| ReviewTarget {
                    kind: &target.0,
                    id: &target.1,
                    revision: target.2,
                }),
            )
            .map_err(Flow::original)?;
        journal
            .set_review_state(&candidate.id, &semantics.review_state)
            .map_err(Flow::original)
    })();
    let settled = boundary::settle_with(
        tx,
        result,
        "write",
        explicit_settlement,
        true,
        AiError::from,
        |_| AiError::TemporaryUnavailable,
    );
    confirm_review(db, settled, &candidate, explicit_settlement)
}

pub(crate) fn confirm_review<P>(
    db: &Connection,
    settled: boundary::Settlement<P, (), AiError>,
    candidate: &AiCandidate,
    explicit_settlement: bool,
) -> Result<AiCandidate, Flow<AiError, P>> {
    match boundary::confirm_with(db, settled, explicit_settlement, |_| {
        AiError::TemporaryUnavailable
    }) {
        Ok(()) => ReviewFacts::new(db)
            .candidate(&candidate.id)
            .map_err(Flow::original),
        Err(BoundaryError::Pause(pause)) => Err(Flow::Need(pause.need())),
        Err(BoundaryError::Failure(failure)) if explicit_settlement && failure.needs_recovery() => {
            Err(Flow::original(AiError::GenerationStorageUnknown(Box::new(
                super::generation_storage_failure::GenerationStorageUnknown::from_failure(
                    &candidate.run_id,
                    "candidate_review",
                    Some(candidate.id.clone()),
                    failure,
                ),
            ))))
        }
        Err(BoundaryError::Failure(failure)) => Err(Flow::Fail(failure.failure)),
    }
}
