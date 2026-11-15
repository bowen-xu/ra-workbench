//! Original research validation/messages over the selected request-bound Native policy.
use super::*;
use crate::ai::{
    candidate_contracts::AiEvidenceBundle,
    error::AiError,
    generation_support::{AiRun, GenerationMessage, GenerationRole, research_policy_input},
};
impl LiteratureTraceService {
    pub(crate) async fn validate_research_evidence(
        &self,
        key: &str,
        run: &AiRun,
        bundle: &AiEvidenceBundle,
    ) -> Result<(), AiError> {
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
                WorkflowInputV1::ResearchEvidence {
                    input: Box::new(research_policy_input(run, bundle)),
                },
            )
            .await
            .map_err(template::technical)?;
        match decision {
            WorkflowDecisionV1::Validated { rejection: None } => Ok(()),
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => Err(template::rejected(error, &origin)),
            _ => Err(template::identified(
                "plugin_host_response_mismatch",
                "研究证据校验结果种类不匹配。",
                &origin,
            )),
        }
    }
    pub(crate) async fn research_messages(
        &self,
        key: &str,
        run: &AiRun,
        bundle: &AiEvidenceBundle,
    ) -> Result<Vec<GenerationMessage>, AiError> {
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
                WorkflowInputV1::ResearchPrompt {
                    input: Box::new(research_policy_input(run, bundle)),
                },
            )
            .await
            .map_err(template::technical)?;
        let WorkflowDecisionV1::ResearchPrompt { result } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "研究消息结果种类不匹配。",
                &origin,
            ));
        };
        let prompt = result.map_err(|error| template::rejected(error, &origin))?;
        Ok(vec![
            GenerationMessage {
                role: GenerationRole::System,
                content: prompt.system_text,
            },
            GenerationMessage {
                role: GenerationRole::User,
                content: prompt.user_text,
            },
        ])
    }
}
