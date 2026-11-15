//! Render only the supplied frozen prompt facts on the selected literature host.
use super::*;
use research_workbench_native_sdk::analysis_workflow::{
    AnalysisWorkflowError,
    prompt::{PaperTaskPrompt, TaskPromptInputV1},
};

impl LiteratureTraceService {
    pub(crate) async fn render_analysis_task_prompt(
        &self,
        key: &str,
        input: TaskPromptInputV1,
    ) -> NativeResult<(
        Result<PaperTaskPrompt, AnalysisWorkflowError>,
        DecisionOrigin,
    )> {
        let input_cbor_base64 = tokio::task::spawn_blocking(move || encode_payload(&input))
            .await
            .map_err(|_| {
                Self::error(
                    "plugin_host_source_worker_unsettled",
                    "论文分析任务提示准备未确认完成。",
                )
                .package(key)
            })?
            .map_err(|_| {
                Self::error(
                    "plugin_host_source_input_invalid",
                    "论文分析任务提示输入无法编码。",
                )
                .package(key)
            })?;
        let (decision, origin) = self
            .analysis_workflow_decision(key, WorkflowInputV1::RenderPrompt { input_cbor_base64 })
            .await?;
        decode_prompt(decision, origin).await
    }

    pub(crate) async fn render_analysis_attempt_prompt(
        &self,
        key: &str,
        messages: Vec<
            research_workbench_native_sdk::analysis_history::contracts::PaperAnalysisMessage,
        >,
    ) -> NativeResult<(
        Result<PaperTaskPrompt, AnalysisWorkflowError>,
        DecisionOrigin,
    )> {
        let (decision, origin) = self
            .analysis_workflow_decision(key, WorkflowInputV1::RenderAttemptMessages { messages })
            .await?;
        decode_prompt(decision, origin).await
    }
}

async fn decode_prompt(
    decision: WorkflowDecisionV1,
    origin: DecisionOrigin,
) -> NativeResult<(
    Result<PaperTaskPrompt, AnalysisWorkflowError>,
    DecisionOrigin,
)> {
    let WorkflowDecisionV1::Prompt { result_cbor_base64 } = decision else {
        return Err(prompt_error(
            "plugin_host_response_mismatch",
            "论文分析任务提示结果种类不匹配。",
            &origin,
        ));
    };
    let result = tokio::task::spawn_blocking(move || {
        decode_payload::<Result<PaperTaskPrompt, AnalysisWorkflowError>>(&result_cbor_base64)
    })
    .await
    .map_err(|_| {
        prompt_error(
            "plugin_host_source_worker_unsettled",
            "论文分析任务提示结果解码未确认完成。",
            &origin,
        )
    })?
    .map_err(|_| {
        prompt_error(
            "plugin_host_response_mismatch",
            "论文分析任务提示结果格式不匹配。",
            &origin,
        )
    })?;
    Ok((result, origin))
}

fn prompt_error(code: &str, message: &str, origin: &DecisionOrigin) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error(code, message).package(&origin.package_key);
    error.instance_id = Some(origin.instance_id.clone());
    error.request_id = Some(origin.native_request_id.clone());
    error
}
