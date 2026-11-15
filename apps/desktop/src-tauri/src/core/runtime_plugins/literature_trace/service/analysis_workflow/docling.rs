//! Original Docling projections over Core-owned source and image observations.
use super::*;
use crate::ai::error::AiError;
use research_workbench_native_sdk::{
    analysis_workflow::{AnalysisWorkflowError, docling::*},
    generation::provider::ProviderInput,
};
mod context;
mod evidence;

impl LiteratureTraceService {
    pub(crate) async fn project_docling_evidence_row(
        &self,
        key: &str,
        input: DoclingRowInputV1,
    ) -> Result<(DoclingRowDecisionV1, DecisionOrigin), AiError> {
        let input_cbor_base64 = tokio::task::spawn_blocking(move || encode_payload(&input))
            .await
            .map_err(|_| input_error(key, "plugin_host_source_worker_unsettled"))?
            .map_err(|_| input_error(key, "plugin_host_source_input_invalid"))?;
        let (decision, origin) = self
            .analysis_workflow_decision(key, WorkflowInputV1::DoclingRow { input_cbor_base64 })
            .await
            .map_err(template::technical)?;
        let WorkflowDecisionV1::DoclingRow { decision } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "Docling 证据结果种类不匹配。",
                &origin,
            ));
        };
        Ok((decision, origin))
    }

    pub(crate) async fn render_docling_overview_prompt(
        &self,
        key: &str,
        input: DoclingPromptInputV1,
    ) -> Result<(ProviderInput, DecisionOrigin), AiError> {
        let input_cbor_base64 = tokio::task::spawn_blocking(move || encode_payload(&input))
            .await
            .map_err(|_| input_error(key, "plugin_host_source_worker_unsettled"))?
            .map_err(|_| input_error(key, "plugin_host_source_input_invalid"))?;
        let (decision, origin) = self
            .analysis_workflow_decision(key, WorkflowInputV1::DoclingPrompt { input_cbor_base64 })
            .await
            .map_err(template::technical)?;
        let WorkflowDecisionV1::DoclingPrompt { result_cbor_base64 } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "Docling 提取提示结果种类不匹配。",
                &origin,
            ));
        };
        let result = tokio::task::spawn_blocking(move || {
            decode_payload::<Result<ProviderInput, AnalysisWorkflowError>>(&result_cbor_base64)
        })
        .await
        .map_err(|_| {
            template::identified(
                "plugin_host_source_worker_unsettled",
                "Docling 提取提示解码未确认完成。",
                &origin,
            )
        })?
        .map_err(|_| {
            template::identified(
                "plugin_host_response_mismatch",
                "Docling 提取提示结果格式不匹配。",
                &origin,
            )
        })?;
        let result = result.map_err(|error| template::rejected(error, &origin))?;
        Ok((result, origin))
    }
}

fn input_error(key: &str, code: &str) -> AiError {
    template::technical(
        LiteratureTraceService::error(code, "Docling 提取策略输入准备失败。").package(key),
    )
}
