//! Shared request-bound knowledge observations. Storage and operation owners stay in Core.
use super::concept_capture::{identified, technical};
use super::*;
use crate::{
    ai::{error::AiError, structured_resource_validation::DecisionOrigin},
    runtime_plugins::{
        literature_concepts::ConceptResources,
        literature_knowledge_promotion::{KnowledgePolicy, PromotionError},
    },
};
use research_workbench_native_sdk::{
    concepts::Concept,
    knowledge_promotion::{
        KnowledgeValidationDecisionV1 as Decision, KnowledgeValidationInputV1 as Input,
        algorithms::PaperFoundationSimilarity, foundations::FoundationRejectionV1,
        manual::DefinitionRevisionRejectionV1, relation::RelationRevisionRejectionV1,
    },
    paper_context_wire::decode_payload,
};
struct Observation {
    input: Input,
    decision: Decision,
    origin: DecisionOrigin,
}
pub(super) struct NativeKnowledgePolicy {
    observations: Vec<Observation>,
    semantic: Option<ConceptResources>,
    source_message: &'static str,
}
impl KnowledgePolicy for NativeKnowledgePolicy {
    type Pause = (usize, Box<Input>);
    fn decide(
        &mut self,
        ordinal: usize,
        input: Input,
    ) -> Result<Decision, PromotionError<Self::Pause>> {
        let Some(observed) = self.observations.get(ordinal) else {
            return Err(PromotionError::Need((ordinal, Box::new(input))));
        };
        if observed.input != input {
            return Err(PromotionError::Failed(identified(
                "plugin_host_source_changed",
                self.source_message,
                &observed.origin,
            )));
        }
        if let Some(message) =
            crate::runtime_plugins::literature_knowledge_promotion::validation_rejection(
                &observed.decision,
            )
        {
            return Err(PromotionError::Failed(identified(
                "invalid_input",
                &message,
                &observed.origin,
            )));
        }
        if let Decision::FoundationComparison { result } = &observed.decision {
            let encoded = result.as_ref().map_err(|message| {
                PromotionError::Failed(identified(
                    "plugin_host_result_encoding_failed",
                    message,
                    &observed.origin,
                ))
            })?;
            let decoded: Result<Vec<PaperFoundationSimilarity>, FoundationRejectionV1> =
                decode_payload(encoded).map_err(|error| {
                    PromotionError::Failed(identified(
                        "plugin_host_response_mismatch",
                        &error.to_string(),
                        &observed.origin,
                    ))
                })?;
            if let Err(reason) = decoded {
                use crate::ai::knowledge_contracts::KnowledgeCommandError;
                use crate::ai::knowledge_contracts::KnowledgeError;
                let (code, error) = match reason {
                    FoundationRejectionV1::NotFound => ("not_found", KnowledgeError::NotFound),
                    FoundationRejectionV1::InvalidInput { message } => {
                        ("invalid_input", KnowledgeError::InvalidInput(message))
                    }
                };
                let mut failure = KnowledgeCommandError::from(error)
                    .into_native()
                    .package(&observed.origin.package_key);
                failure.instance_id = Some(observed.origin.instance_id.clone());
                failure.request_id = Some(observed.origin.native_request_id.clone());
                return Err(PromotionError::Failed(AiError::NativeWorkflow {
                    code,
                    failure,
                }));
            }
        }
        if let Decision::DefinitionRevision {
            rejection: Some(reason),
        } = &observed.decision
        {
            let (code, message, current_revision) = match reason {
                DefinitionRevisionRejectionV1::NewExpectedRevision => (
                    "invalid_input",
                    "新建定义不能指定 expectedRevision".to_owned(),
                    None,
                ),
                DefinitionRevisionRejectionV1::Conflict { current_revision } => (
                    "revision_conflict",
                    format!("对象版本冲突；当前版本为 {current_revision}"),
                    Some(*current_revision),
                ),
            };
            let mut failure = Self::revision_failure(code, &message, &observed.origin);
            failure.current_revision = current_revision;
            return Err(PromotionError::Failed(AiError::NativeWorkflow {
                code,
                failure,
            }));
        }
        if let Decision::RelationRevision {
            rejection: Some(reason),
        } = &observed.decision
        {
            let (code, message, current_revision) = match reason {
                RelationRevisionRejectionV1::NewExpectedRevision => (
                    "invalid_input",
                    "新建关系不能指定现有 revision".into(),
                    None,
                ),
                RelationRevisionRejectionV1::MissingExpectedRevision => (
                    "invalid_input",
                    "修改关系必须提供 expectedRevision".into(),
                    None,
                ),
                RelationRevisionRejectionV1::Conflict { current_revision } => (
                    "revision_conflict",
                    format!("对象版本冲突；当前版本为 {current_revision}"),
                    Some(*current_revision),
                ),
            };
            let mut failure = Self::revision_failure(code, &message, &observed.origin);
            failure.current_revision = current_revision;
            return Err(PromotionError::Failed(AiError::NativeWorkflow {
                code,
                failure,
            }));
        }
        Ok(observed.decision.clone())
    }
    fn register_semantic(
        &mut self,
        tx: &rusqlite::Transaction<'_>,
        concept: &Concept,
    ) -> Result<(), PromotionError<Self::Pause>> {
        let semantic = self
            .semantic
            .as_ref()
            .ok_or(PromotionError::Failed(AiError::TemporaryUnavailable))?;
        crate::runtime_plugins::literature_concepts::register_accepted_semantic(
            tx, semantic, concept,
        )
        .map_err(|failure| PromotionError::Failed(technical(failure.native())))
    }
}
impl NativeKnowledgePolicy {
    pub(super) fn new(semantic: Option<ConceptResources>, source_message: &'static str) -> Self {
        Self {
            observations: Vec::new(),
            semantic,
            source_message,
        }
    }
    pub(super) fn remember(&mut self, input: Input, decision: Decision, origin: DecisionOrigin) {
        self.observations.push(Observation {
            input,
            decision,
            origin,
        });
    }

    fn revision_failure(
        code: &'static str,
        message: &str,
        origin: &DecisionOrigin,
    ) -> Box<NativeHostError> {
        let mut failure = LiteratureTraceService::error(code, message).package(&origin.package_key);
        failure.instance_id = Some(origin.instance_id.clone());
        failure.request_id = Some(origin.native_request_id.clone());
        failure
    }
}
