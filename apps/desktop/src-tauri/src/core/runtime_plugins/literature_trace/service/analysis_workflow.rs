//! Pure workflow decisions use the existing literature host and request lifetime.
use super::*;
use crate::ai::structured_resource_validation::DecisionOrigin;
use research_workbench_native_sdk::{
    analysis_workflow::wire::*,
    catalog::{CatalogInput, CatalogOutput},
    paper_context_wire::{decode_payload, encode_payload},
};
mod attempt;
mod candidate;
mod creation;
mod docling;
mod failure;
mod overview;
mod portrait;
mod preparation;
mod prompt;
mod recovery;
mod research;
mod review;
mod suggestion;
pub(super) mod template;
pub(crate) use attempt::StartAnalysisAttempt;
pub(crate) use failure::AttemptFailure;

impl LiteratureTraceService {
    #[cfg(not(unix))]
    pub(crate) async fn analysis_workflow_decision(
        &self,
        _key: &str,
        _input: WorkflowInputV1,
    ) -> NativeResult<(WorkflowDecisionV1, DecisionOrigin)> {
        Err(Self::error(
            "plugin_host_platform_unsupported",
            "此平台尚未提供文献宿主。",
        ))
    }

    #[cfg(unix)]
    pub(crate) async fn analysis_workflow_decision(
        &self,
        key: &str,
        input: WorkflowInputV1,
    ) -> NativeResult<(WorkflowDecisionV1, DecisionOrigin)> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let (encoded, expected) = tokio::task::spawn_blocking(move || {
            package::accept_analysis_workflow(&catalog, &selected)?;
            let encoded = encode_payload(&input).map_err(|_| {
                Self::error(
                    "plugin_host_source_input_invalid",
                    "论文分析工作流输入无法编码。",
                )
            })?;
            Ok::<_, Box<NativeHostError>>((encoded, input))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_source_worker_unsettled",
                "论文分析工作流输入准备未确认完成。",
            )
        })??;
        let mut slot = self.host.lock().await;
        self.available()?;
        let result = async {
            let host = self.ensure_host(&mut slot, key).await?;
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available()?;
            let before = host.request_identity();
            let request = CatalogInput::DecideAnalysisWorkflow {
                input: Box::new(WorkflowWireInputV1 { cbor_base64: encoded.clone() }),
            };
            let output = tokio::select! {
                biased;
                _ = &mut stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭论文分析工作流。")),
                output = host.catalog_call(request, Duration::from_secs(60)) => output,
            }.map_err(|mut error| {
                match host.request_identity() {
                    Some((instance, request)) if before.as_ref() != Some(&(instance.clone(), request.clone())) => {
                        error.instance_id = Some(instance);
                        error.request_id = Some(request);
                    }
                    _ => error.request_id = None,
                }
                error
            })?;
            let CatalogOutput::AnalysisWorkflow { outcome } = output else {
                return Err(host.error("plugin_host_response_mismatch", "论文分析工作流结果种类不匹配。"));
            };
            let decision = tokio::task::spawn_blocking(move || {
                if outcome.input.cbor_base64 != encoded { return Err(()); }
                let decision: WorkflowDecisionV1 = decode_payload(&outcome.decision_cbor_base64).map_err(|_| ())?;
                if !expected.matches_decision(&decision) { return Err(()); }
                Ok(decision)
            }).await.map_err(|_| host.error("plugin_host_source_worker_unsettled", "论文分析工作流结果解码未确认完成。"))?
                .map_err(|_| host.error("plugin_host_response_mismatch", "论文分析工作流结果与本次输入不匹配。"))?;
            let Some((instance_id, native_request_id)) = host.request_identity() else {
                return Err(host.error("plugin_host_response_mismatch", "论文分析工作流结果缺少本次身份。"));
            };
            Ok((decision, DecisionOrigin { package_key: key.into(), instance_id, native_request_id }))
        }.await;
        self.finish_domain(&mut slot, result).await
    }
}

#[cfg(test)]
pub(crate) use overview::test_sources::preview as preview_for_test;
