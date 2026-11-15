//! Original paper-concept item qualification and atomic acceptance through finite rules.
use super::{
    candidate_contracts::CandidatePayload,
    error::AiError,
    item_review::ItemReviewAdapter,
    review_support::{
        AiCandidate, AiSuggestionResolutionDraft, ReviewFacts, ReviewJournal, ReviewTarget,
        current_payload_source, validate_citation_readiness,
    },
    structured_resource_validation::Flow,
};
use research_workbench_native_sdk::candidate_review::{
    items::AiSuggestionItemDecisionDraft,
    paper::{PaperItemV1, PaperReviewDecisionV1 as Decision, PaperReviewInputV1 as Input},
};
use rusqlite::Connection;
pub(crate) trait PaperReviewAdapter: ItemReviewAdapter {
    fn paper(&mut self, input: Input) -> Result<Decision, Flow<AiError, Self::Pause>>;
}
fn checked<P>(decision: Decision) -> Result<(), Flow<AiError, P>> {
    match decision {
        Decision::Checked { rejection: None } => Ok(()),
        Decision::Checked {
            rejection: Some(error),
        } => Err(Flow::original(error.into())),
        _ => Err(Flow::original(AiError::TemporaryUnavailable)),
    }
}
pub(crate) fn decide_with<A: PaperReviewAdapter>(
    db: &mut Connection,
    draft: AiSuggestionItemDecisionDraft,
    explicit_settlement: bool,
    adapter: &mut A,
) -> Result<AiCandidate, Flow<AiError, A::Pause>> {
    let hash = match adapter.paper(Input::Draft {
        draft: draft.clone(),
    })? {
        Decision::Draft { result } => result.map_err(|e| Flow::original(e.into()))?,
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    if let Some(existing) = ReviewFacts::new(db)
        .item_decision(&draft.idempotency_key)
        .map_err(Flow::original)?
    {
        checked(adapter.paper(Input::Replay {
            draft: draft.clone(),
            request_hash: hash,
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
    checked(adapter.paper(Input::Candidate {
        revision: draft.candidate_revision,
        current_revision: candidate.current_revision,
        review_state: candidate.review_state.clone(),
    })?)?;
    let (value, _) = current_payload_source(&candidate).map_err(Flow::original)?;
    let payload = adapter.decode(&candidate, value)?;
    let editor = candidate
        .revisions
        .iter()
        .find(|r| r.revision == candidate.current_revision)
        .ok_or_else(|| Flow::original(AiError::InvalidInput("找不到候选当前版本".into())))?
        .editor_kind
        .clone();
    let value = serde_json::to_value(&payload)
        .map_err(|_| Flow::original(AiError::TemporaryUnavailable))?;
    let item = match adapter.paper(Input::Select {
        payload: value,
        suggestion_id: draft.suggestion_id.clone(),
    })? {
        Decision::Selected { result } => result.map_err(|e| Flow::original(e.into()))?,
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    let semantics = match adapter.paper(Input::Action {
        draft: draft.clone(),
        editor_kind: editor.clone(),
        item: *item.clone(),
    })? {
        Decision::Action { result } => result.map_err(|e| Flow::original(e.into()))?,
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    if semantics.is_accept {
        for id in item.citation_ids() {
            validate_citation_readiness(&candidate, &payload, Some(&id)).map_err(Flow::original)?;
        }
    }
    let evidence_ids = match adapter.paper(Input::Evidence {
        item: *item.clone(),
    })? {
        Decision::Evidence { evidence_ids } => evidence_ids,
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    if draft.action == "accept_unchanged" {
        let inspected = ReviewFacts::new(db)
            .inspected_keys(&draft)
            .map_err(Flow::original)?;
        checked(adapter.paper(Input::Inspection {
            inspected: evidence_ids.iter().any(|key| inspected.contains(key)),
        })?)?;
    }
    let paper_id = match adapter.paper(Input::Paper {
        paper_ids: candidate
            .evidence_bundle
            .items
            .iter()
            .map(|e| e.paper_id.clone())
            .collect(),
    })? {
        Decision::Paper { result } => result.map_err(|e| Flow::original(e.into()))?,
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    let tx = db.transaction().map_err(|e| Flow::original(e.into()))?;
    let result = (|| {
        let target = if semantics.is_accept {
            if let (
                PaperItemV1::Relation(relation),
                Some(AiSuggestionResolutionDraft::AssertRelation {
                    subject_concept_id,
                    object_concept_id,
                    ..
                }),
            ) = (item.as_ref(), draft.resolution.as_ref())
            {
                let mut accepted = ReviewFacts::new(&tx)
                    .accepted_concept_targets(&candidate)
                    .map_err(Flow::original)?
                    .into_iter()
                    .collect::<Vec<_>>();
                accepted.sort();
                let concept_ids = match &payload {
                    CandidatePayload::PaperConceptMap(map) => {
                        map.concepts.iter().map(|c| c.id.clone()).collect()
                    }
                    _ => {
                        return Err(Flow::original(AiError::InvalidInput(
                            "候选不是论文概念图".into(),
                        )));
                    }
                };
                checked(adapter.paper(Input::Endpoints {
                    concept_ids,
                    relation: relation.clone(),
                    subject_id: subject_concept_id.clone(),
                    object_id: object_concept_id.clone(),
                    accepted,
                })?)?;
            }
            let plan = match adapter.paper(Input::Promotion {
                item: *item,
                paper_id,
                candidate_id: candidate.id.clone(),
                candidate_revision: draft.candidate_revision,
                resolution: draft.resolution.clone(),
                sources: candidate
                    .evidence_bundle
                    .items
                    .iter()
                    .map(|e| (e.citation_key.clone(), e.anchor_id.clone()))
                    .collect(),
            })? {
                Decision::Promotion { result } => result.map_err(|e| Flow::original(e.into()))?,
                _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
            };
            Some(adapter.promote(&tx, *plan, &semantics.provenance)?)
        } else {
            None
        };
        let journal = ReviewJournal::new(&tx);
        journal
            .append_item(
                &draft,
                &hash,
                &semantics.resolution_kind,
                target.as_ref().map(|(kind, id, revision)| ReviewTarget {
                    kind,
                    id,
                    revision: *revision,
                }),
            )
            .map_err(Flow::original)?;
        let (terminal, accepted) = journal
            .facts()
            .item_counts(&candidate.id, candidate.current_revision)
            .map_err(Flow::original)?;
        let total = match payload {
            CandidatePayload::PaperConceptMap(map) => map.concepts.len() + map.relations.len(),
            _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
        };
        i64::try_from(total).map_err(|_| Flow::original(AiError::TemporaryUnavailable))?;
        let state = match adapter.paper(Input::State {
            total,
            terminal,
            accepted,
            editor_kind: editor,
            action: draft.action.clone(),
        })? {
            Decision::State { state } => state,
            _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
        };
        journal
            .set_review_state(&candidate.id, &state)
            .map_err(Flow::original)
    })();
    let settled = crate::hbs_storage::boundary::settle_with(
        tx,
        result,
        "write",
        explicit_settlement,
        true,
        AiError::from,
        |_| AiError::TemporaryUnavailable,
    );
    super::generic_review::confirm_review(db, settled, &candidate, explicit_settlement)
}
