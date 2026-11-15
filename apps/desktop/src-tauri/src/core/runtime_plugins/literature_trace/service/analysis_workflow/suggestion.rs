//! Original two-role suggestion messages from the selected finite domain policy.
use super::*;
use crate::ai::{
    candidate_contracts::AiEvidenceBundle,
    error::AiError,
    generation_support::{
        AiRun, GenerationMessage, GenerationRole, preview_frozen_overview_evidence,
    },
};

impl LiteratureTraceService {
    pub(crate) async fn suggestion_messages(
        &self,
        key: &str,
        run: &AiRun,
        bundle: &AiEvidenceBundle,
    ) -> Result<Vec<GenerationMessage>, AiError> {
        let kind = run
            .suggestion_candidate_kind
            .as_ref()
            .ok_or_else(|| AiError::InvalidInput("建议运行缺少冻结候选类型".into()))?;
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
                WorkflowInputV1::SuggestionPrompt {
                    candidate_kind: kind.clone(),
                    user_goal: run.user_goal.clone(),
                    evidence: preview_frozen_overview_evidence(bundle),
                },
            )
            .await
            .map_err(template::technical)?;
        let WorkflowDecisionV1::SuggestionPrompt { result } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "建议消息结果种类不匹配。",
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
