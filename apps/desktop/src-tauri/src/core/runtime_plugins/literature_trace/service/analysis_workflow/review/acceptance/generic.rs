//! Replay original historical reading-unit promotion within the original review transaction.
use super::*;
use research_workbench_native_sdk::candidate_review::generic::{
    GenericPromotionInputV1, GenericPromotionPlanV1,
};
impl ReviewPolicy {
    pub(super) fn promote_generic(
        &mut self,
        tx: &Transaction<'_>,
        plan: &ReviewPlan<'_>,
    ) -> ReviewPromotion<Need> {
        // The original review collects a set. Stable ordering binds retries to that same set.
        let mut anchor_ids = plan.anchor_ids.to_vec();
        anchor_ids.sort();
        let input = GenericPromotionInputV1 {
            payload: serde_json::to_value(plan.payload)
                .map_err(|_| Flow::original(AiError::TemporaryUnavailable))?,
            reading_unit_title: plan.draft.reading_unit_title.clone(),
            project_id: plan.draft.project_id.clone(),
            paper_id: plan.paper_id.into(),
            anchor_ids,
            provenance_kind: plan.provenance.expect("promotion has provenance").into(),
            candidate_id: plan.candidate.id.clone(),
            candidate_revision: plan.draft.candidate_revision,
        };
        let ReviewDecisionV1::GenericPromotion {
            result: Ok(prepared),
        } = self.review(ReviewInputV1::GenericPromotion {
            input: Box::new(input),
        })?
        else {
            return Err(Flow::original(AiError::TemporaryUnavailable));
        };
        let units = match *prepared {
            GenericPromotionPlanV1::Portrait {
                candidate_id,
                candidate_revision,
            } => {
                return Ok(Some((
                    "paper_portrait".into(),
                    candidate_id,
                    candidate_revision,
                )));
            }
            GenericPromotionPlanV1::ReadingUnits { units } => units,
        };
        let Some(sql) = self.reading_units.take() else {
            return Err(Flow::Need(Need::ReadingUnits));
        };
        let result = (|| {
            let mut first = None;
            for (ordinal, unit) in units.iter().enumerate() {
                let target = sql.apply(tx, unit, &mut |draft| match self.review(
                    ReviewInputV1::ReadingUnitValidation {
                        ordinal,
                        draft: Box::new(draft.clone()),
                    },
                )? {
                    ReviewDecisionV1::ReadingUnitValidation { rejection: None } => Ok(()),
                    _ => Err(Flow::original(AiError::TemporaryUnavailable)),
                })?;
                if first.is_none() {
                    first = Some(("reading_unit".into(), target.0, target.1));
                }
            }
            Ok(first)
        })();
        self.reading_units = Some(sql);
        result
    }
}
