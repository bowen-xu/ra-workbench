//! Finite item review shares the original Core review owner, facts and transaction.
use super::*;
use crate::ai::item_review::ItemReviewAdapter;
use crate::runtime_plugins::literature_knowledge_promotion::{
    KnowledgeIo, KnowledgePolicy, PromotionError,
};
use research_workbench_native_sdk::{
    candidate_contracts::CandidateValidationError,
    candidate_review::items::{ItemReviewDecisionV1 as Decision, ItemReviewInputV1 as Input},
    concepts::Concept,
    knowledge_promotion::{
        KnowledgePromotionV1, KnowledgeValidationDecisionV1, KnowledgeValidationInputV1,
    },
};
impl ItemReviewAdapter for ReviewPolicy {
    type Pause = Need;
    fn item(&mut self, input: Input) -> Result<Decision, Flow<AiError, Need>> {
        let request = WorkflowInputV1::SuggestionReview {
            input: Box::new(input),
        };
        let decision = self.observed(request.clone())?;
        let origin = &self
            .observations
            .iter()
            .find(|entry| same_rule(&entry.input, &request))
            .expect("observed item rule")
            .origin;
        let WorkflowDecisionV1::SuggestionReview { decision } = decision else {
            return Err(Flow::original(template::identified(
                "plugin_host_response_mismatch",
                "建议审阅结果种类不匹配。",
                origin,
            )));
        };
        let rejection = match &decision {
            Decision::Draft { result } => result.as_ref().err().cloned(),
            Decision::Checked { rejection } => rejection.clone(),
            Decision::Selected { result } => result.as_ref().err().cloned(),
            Decision::Action { result } => result.as_ref().err().cloned(),
            Decision::Anchors { result } => result.as_ref().err().cloned(),
            Decision::Promotion { result } => result.as_ref().err().cloned(),
            Decision::Knowledge { decision } => {
                crate::runtime_plugins::literature_knowledge_promotion::validation_rejection(
                    decision,
                )
                .map(CandidateValidationError::InvalidInput)
            }
            Decision::State { .. } => None,
        };
        if let Some(error) = rejection {
            return Err(Flow::original(template::rejected(error, origin)));
        }
        Ok(decision)
    }
    fn decode(
        &mut self,
        candidate: &AiCandidate,
        value: serde_json::Value,
    ) -> Result<CandidatePayload, Flow<AiError, Need>> {
        GenericReviewAdapter::decode(self, candidate, value)
    }
    fn promote(
        &mut self,
        tx: &Transaction<'_>,
        plan: KnowledgePromotionV1,
        provenance: &str,
    ) -> Result<(String, String, i64), Flow<AiError, Need>> {
        let paper = matches!(&plan, KnowledgePromotionV1::PaperConcept(_));
        let Some(sql) = self.knowledge.take() else {
            return Err(Flow::Need(if paper {
                Need::PaperKnowledge
            } else {
                Need::Knowledge
            }));
        };
        if paper && !sql.has_paper() {
            self.knowledge = Some(sql);
            return Err(Flow::Need(Need::PaperKnowledge));
        }
        let result = KnowledgeIo::new(tx, &sql, self).promote(plan, provenance);
        self.knowledge = Some(sql);
        result
            .map(|target| (target.kind.into(), target.id, target.revision))
            .map_err(flow)
    }
}
impl KnowledgePolicy for ReviewPolicy {
    type Pause = Need;
    fn decide(
        &mut self,
        ordinal: usize,
        input: KnowledgeValidationInputV1,
    ) -> Result<KnowledgeValidationDecisionV1, PromotionError<Need>> {
        match ItemReviewAdapter::item(
            self,
            Input::Knowledge {
                ordinal,
                input: Box::new(input),
            },
        )
        .map_err(promotion_error)?
        {
            Decision::Knowledge { decision } => Ok(decision),
            _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
    }
    fn register_semantic(
        &mut self,
        tx: &Transaction<'_>,
        concept: &Concept,
    ) -> Result<(), PromotionError<Need>> {
        let resources = self
            .semantic
            .as_ref()
            .ok_or(PromotionError::Need(Need::Knowledge))?;
        crate::runtime_plugins::literature_concepts::register_accepted_semantic(
            tx, resources, concept,
        )
        .map_err(|error| {
            let mut error = error.native();
            error.package_key = Some(resources.package_key().into());
            PromotionError::Failed(AiError::NativeWorkflow {
                code: "storage_error",
                failure: error,
            })
        })
    }
}
fn promotion_error(error: Flow<AiError, Need>) -> PromotionError<Need> {
    match error {
        Flow::Fail(failure) => PromotionError::Failed(failure.error),
        Flow::Need(need) => PromotionError::Need(need),
    }
}
fn flow(error: PromotionError<Need>) -> Flow<AiError, Need> {
    match error {
        PromotionError::Failed(error) => Flow::original(error),
        PromotionError::Need(need) => Flow::Need(need),
    }
}
