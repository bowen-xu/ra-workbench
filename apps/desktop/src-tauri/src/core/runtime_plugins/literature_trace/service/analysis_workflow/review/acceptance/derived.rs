//! Preserve the original matrix-context read and atomic derived-view promotion.
use super::*;
use crate::ai::review_support::ReviewFacts;
use research_workbench_native_sdk::{
    candidate_contracts::DerivedResearchViewCandidatePayload,
    candidate_review::derived::DerivedPromotionInputV1,
};
impl ReviewPolicy {
    pub(super) fn promote_derived(
        &mut self,
        tx: &Transaction<'_>,
        plan: &ReviewPlan<'_>,
        view: &DerivedResearchViewCandidatePayload,
    ) -> ReviewPromotion<Need> {
        let input = DerivedPromotionInputV1 {
            view: view.clone(),
            context: ReviewFacts::new(tx)
                .matrix_context(&plan.candidate.run_id)
                .map_err(Flow::original)?,
            project_id: plan.draft.project_id.clone(),
            anchors_by_key: plan
                .candidate
                .evidence_bundle
                .items
                .iter()
                .map(|item| (item.citation_key.clone(), item.anchor_id.clone()))
                .collect(),
            provenance_kind: plan.provenance.expect("promotion has provenance").into(),
            provider_id: plan.candidate.provider_id.clone(),
            model_id: plan.candidate.model_id.clone(),
            generated_at: plan.candidate.created_at.clone(),
            candidate_id: plan.candidate.id.clone(),
            candidate_revision: plan.draft.candidate_revision,
        };
        let decision = self.review(ReviewInputV1::DerivedPromotion {
            input: Box::new(input),
        })?;
        let ReviewDecisionV1::DerivedPromotion {
            result: Ok(prepared),
        } = decision
        else {
            return Err(Flow::original(AiError::TemporaryUnavailable));
        };
        let Some(sql) = self.derived.take() else {
            return Err(Flow::Need(Need::Derived));
        };
        let result = sql.apply(tx, &prepared, &mut |input| match self.review(
            ReviewInputV1::DerivedValidation {
                input: Box::new(input),
            },
        )? {
            ReviewDecisionV1::DerivedValidation { rejection: None } => Ok(()),
            _ => Err(Flow::original(AiError::TemporaryUnavailable)),
        });
        self.derived = Some(sql);
        result.map(|(id, revision)| Some(("derived_research_view".into(), id, revision)))
    }
}
