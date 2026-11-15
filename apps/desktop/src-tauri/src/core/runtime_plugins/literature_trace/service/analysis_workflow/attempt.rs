//! Original attempt qualification over owned history observations.
use super::*;
use crate::ai::{error::AiError, paper_analysis_contracts::PaperAnalysisMessage};
use research_workbench_native_sdk::analysis_attempt::{
    AnalysisAttemptPlan, AnalysisAttemptWorkflowObservation,
};

mod admission;
mod completion;
mod completion_store;
pub(crate) use admission::StartAnalysisAttempt;
mod source;
mod worker;

impl LiteratureTraceService {
    pub(crate) async fn validate_analysis_attempt_input(
        &self,
        key: &str,
        turn_kind: &str,
        messages: &[PaperAnalysisMessage],
    ) -> Result<DecisionOrigin, AiError> {
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
                WorkflowInputV1::ValidateAttemptInput {
                    turn_kind: turn_kind.into(),
                    contents: messages
                        .iter()
                        .map(|message| message.content.clone())
                        .collect(),
                },
            )
            .await
            .map_err(template::technical)?;
        match decision {
            WorkflowDecisionV1::Validated { rejection: None } => Ok(origin),
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => Err(template::rejected(error, &origin)),
            _ => Err(template::identified(
                "plugin_host_response_mismatch",
                "分析尝试输入校验结果种类不匹配。",
                &origin,
            )),
        }
    }
    pub(crate) async fn plan_analysis_attempt(
        &self,
        key: &str,
        turn_kind: &str,
        workflow: AnalysisAttemptWorkflowObservation,
    ) -> Result<(AnalysisAttemptPlan, DecisionOrigin), AiError> {
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
                WorkflowInputV1::PlanAttempt {
                    turn_kind: turn_kind.into(),
                    workflow,
                },
            )
            .await
            .map_err(template::technical)?;
        let WorkflowDecisionV1::AttemptPlan { result } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "分析尝试资格结果种类不匹配。",
                &origin,
            ));
        };
        let plan = result.map_err(|error| template::rejected(error, &origin))?;
        Ok((plan, origin))
    }
}
