//! Paper-concept rules reuse the same owned review worker and original knowledge writer.
use super::*;
use crate::ai::paper_item_review::PaperReviewAdapter;
use research_workbench_native_sdk::candidate_review::paper::{
    PaperReviewDecisionV1 as Decision, PaperReviewInputV1 as Input,
};
impl PaperReviewAdapter for ReviewPolicy {
    fn paper(&mut self, input: Input) -> Result<Decision, Flow<AiError, Need>> {
        let request = WorkflowInputV1::PaperConceptReview {
            input: Box::new(input),
        };
        let decision = self.observed(request.clone())?;
        let origin = &self
            .observations
            .iter()
            .find(|entry| same_rule(&entry.input, &request))
            .expect("observed paper review")
            .origin;
        let WorkflowDecisionV1::PaperConceptReview { decision } = decision else {
            return Err(Flow::original(template::identified(
                "plugin_host_response_mismatch",
                "论文概念审阅结果种类不匹配。",
                origin,
            )));
        };
        let rejection = match &decision {
            Decision::Draft { result } => result.as_ref().err().cloned(),
            Decision::Checked { rejection } => rejection.clone(),
            Decision::Selected { result } => result.as_ref().err().cloned(),
            Decision::Action { result } => result.as_ref().err().cloned(),
            Decision::Paper { result } => result.as_ref().err().cloned(),
            Decision::Promotion { result } => result.as_ref().err().cloned(),
            Decision::Evidence { .. } | Decision::State { .. } => None,
        };
        if let Some(error) = rejection {
            return Err(Flow::original(template::rejected(error, origin)));
        }
        Ok(decision)
    }
}
