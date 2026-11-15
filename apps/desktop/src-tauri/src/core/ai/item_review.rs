//! Original per-item review reads and transaction, with finite domain decisions.
use super::{
    candidate_contracts::CandidatePayload,
    error::AiError,
    review_support::{
        AiCandidate, ReviewFacts, ReviewJournal, ReviewTarget, current_payload_source,
        validate_citation_readiness,
    },
    structured_resource_validation::Flow,
};
use research_workbench_native_sdk::{
    candidate_review::items::*, knowledge_promotion::KnowledgePromotionV1,
};
use rusqlite::{Connection, Transaction};

pub(crate) trait ItemReviewAdapter {
    type Pause;
    fn item(
        &mut self,
        input: ItemReviewInputV1,
    ) -> Result<ItemReviewDecisionV1, Flow<AiError, Self::Pause>>;
    fn decode(
        &mut self,
        candidate: &AiCandidate,
        value: serde_json::Value,
    ) -> Result<CandidatePayload, Flow<AiError, Self::Pause>>;
    fn promote(
        &mut self,
        tx: &Transaction<'_>,
        plan: KnowledgePromotionV1,
        provenance: &str,
    ) -> Result<(String, String, i64), Flow<AiError, Self::Pause>>;
}
fn checked<P>(decision: ItemReviewDecisionV1) -> Result<(), Flow<AiError, P>> {
    match decision {
        ItemReviewDecisionV1::Checked { rejection: None } => Ok(()),
        ItemReviewDecisionV1::Checked {
            rejection: Some(error),
        } => Err(Flow::original(error.into())),
        _ => Err(Flow::original(AiError::TemporaryUnavailable)),
    }
}
pub(crate) fn decide_with<A: ItemReviewAdapter>(
    db: &mut Connection,
    draft: AiSuggestionItemDecisionDraft,
    explicit_settlement: bool,
    adapter: &mut A,
) -> Result<AiCandidate, Flow<AiError, A::Pause>> {
    let hash = match adapter.item(ItemReviewInputV1::Draft {
        draft: draft.clone(),
    })? {
        ItemReviewDecisionV1::Draft { result } => {
            result.map_err(|error| Flow::original(error.into()))?
        }
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    if let Some(existing) = ReviewFacts::new(db)
        .item_decision(&draft.idempotency_key)
        .map_err(Flow::original)?
    {
        checked(adapter.item(ItemReviewInputV1::Replay {
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
    checked(adapter.item(ItemReviewInputV1::Candidate {
        revision: draft.candidate_revision,
        current_revision: candidate.current_revision,
        review_state: candidate.review_state.clone(),
    })?)?;
    let terminal = ReviewFacts::new(db)
        .item_is_terminal(&draft)
        .map_err(Flow::original)?;
    checked(adapter.item(ItemReviewInputV1::Terminal {
        has_terminal_decision: terminal,
    })?)?;
    let (value, _) = current_payload_source(&candidate).map_err(Flow::original)?;
    let payload = adapter.decode(&candidate, value)?;
    let editor = candidate
        .revisions
        .iter()
        .find(|revision| revision.revision == candidate.current_revision)
        .ok_or_else(|| Flow::original(AiError::InvalidInput("找不到候选当前版本".into())))?
        .editor_kind
        .clone();
    let value = serde_json::to_value(&payload)
        .map_err(|_| Flow::original(AiError::TemporaryUnavailable))?;
    let selected = match adapter.item(ItemReviewInputV1::Select {
        payload: value.clone(),
        suggestion_id: draft.suggestion_id.clone(),
    })? {
        ItemReviewDecisionV1::Selected { result } => {
            result.map_err(|error| Flow::original(error.into()))?
        }
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    let semantics = match adapter.item(ItemReviewInputV1::Action {
        draft: draft.clone(),
        editor_kind: editor.clone(),
        suggestion: selected.clone(),
    })? {
        ItemReviewDecisionV1::Action { result } => {
            result.map_err(|error| Flow::original(error.into()))?
        }
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    if semantics.is_accept {
        validate_citation_readiness(&candidate, &payload, Some(&draft.suggestion_id))
            .map_err(Flow::original)?;
    }
    if draft.action == "accept_unchanged" {
        let inspected = ReviewFacts::new(db)
            .inspected_keys(&draft)
            .map_err(Flow::original)?;
        checked(
            adapter.item(ItemReviewInputV1::Inspection {
                inspected: selected
                    .evidence_ids()
                    .iter()
                    .any(|id| inspected.contains(id)),
            })?,
        )?;
    }
    let anchors = match adapter.item(ItemReviewInputV1::Anchors {
        evidence_ids: selected.evidence_ids().to_vec(),
        sources: candidate
            .evidence_bundle
            .items
            .iter()
            .map(|item| (item.citation_key.clone(), item.anchor_id.clone()))
            .collect(),
    })? {
        ItemReviewDecisionV1::Anchors { result } => {
            result.map_err(|error| Flow::original(error.into()))?
        }
        _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
    };
    let tx = db
        .transaction()
        .map_err(|error| Flow::original(error.into()))?;
    let result = (|| {
        let target = if semantics.is_accept {
            let prepared = match adapter.item(ItemReviewInputV1::Promotion {
                suggestion: selected,
                resolution: draft.resolution.clone(),
                anchor_ids: anchors,
            })? {
                ItemReviewDecisionV1::Promotion { result } => {
                    result.map_err(|error| Flow::original(error.into()))?
                }
                _ => return Err(Flow::original(AiError::TemporaryUnavailable)),
            };
            Some(adapter.promote(&tx, *prepared, &semantics.provenance)?)
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
        let state = match adapter.item(ItemReviewInputV1::State {
            total: value["suggestions"].as_array().map_or(0, Vec::len),
            terminal,
            accepted,
            editor_kind: editor,
            action: draft.action.clone(),
        })? {
            ItemReviewDecisionV1::State { state } => state,
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
