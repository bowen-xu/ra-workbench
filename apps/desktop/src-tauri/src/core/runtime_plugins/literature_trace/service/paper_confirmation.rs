//! Finite policy exchange on the existing owned literature host.
mod runner;
use super::*;
use crate::ai::structured_resource_validation::DecisionOrigin;
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    paper_confirmation::wire::{
        PaperConfirmationInputV1, PaperConfirmationResultV1, PaperConfirmationWireInputV1,
    },
    paper_context_wire::{decode_payload, encode_payload},
};

impl LiteratureTraceService {
    #[cfg(unix)]
    pub(super) async fn paper_confirmation_stage(
        &self,
        host: &mut NativeHost,
        key: &str,
        input: PaperConfirmationInputV1,
    ) -> NativeResult<(PaperConfirmationResultV1, DecisionOrigin)> {
        let phase = stage_name(&input);
        let encoded = tokio::task::spawn_blocking(move || encode_payload(&input))
            .await
            .map_err(|_| {
                host.error(
                    "plugin_host_source_worker_unsettled",
                    "字段确认输入编码未确认完成。",
                )
            })?
            .map_err(|_| {
                host.error("plugin_host_source_input_invalid", "字段确认输入无法编码。")
            })?;
        let expected = encoded.clone();
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let before = host.request_identity();
        let request = CatalogInput::PreparePaperConfirmation {
            input: Box::new(PaperConfirmationWireInputV1 {
                cbor_base64: encoded,
            }),
        };
        let output = tokio::select! {
            biased;
            _ = &mut stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭字段确认。")),
            output = host.catalog_call(request, Duration::from_secs(60)) => output,
        }
        .map_err(|mut error| {
            match host.request_identity() {
                Some((instance, request))
                    if before.as_ref() != Some(&(instance.clone(), request.clone())) =>
                {
                    error.instance_id = Some(instance);
                    error.request_id = Some(request);
                }
                _ => error.request_id = None,
            }
            error
        })?;
        let CatalogOutput::PaperConfirmation { outcome } = output else {
            return Err(host.error("plugin_host_response_mismatch", "字段确认结果种类不匹配。"));
        };
        let result = tokio::task::spawn_blocking(move || {
            if outcome.input.cbor_base64 != expected {
                return Err("input_echo".to_owned());
            }
            let input: PaperConfirmationInputV1 =
                decode_payload(&expected).map_err(|error| format!("input_decode {error}"))?;
            let result: PaperConfirmationResultV1 = decode_payload(&outcome.result_cbor_base64)
                .map_err(|error| format!("result_decode {error}"))?;
            if !input.matches_result(&result) {
                return Err("decision_kind".to_owned());
            }
            Ok(result)
        })
        .await
        .map_err(|_| {
            host.error(
                "plugin_host_source_worker_unsettled",
                "字段确认结果解码未确认完成。",
            )
        })?
        .map_err(|reason| {
            host.error(
                "plugin_host_response_mismatch",
                format!("字段确认的 {phase} 结果与本次输入不匹配。校验 {reason}。"),
            )
        })?;
        let Some((instance_id, native_request_id)) = host.request_identity() else {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "字段确认结果缺少本次身份。",
            ));
        };
        Ok((
            result,
            DecisionOrigin {
                package_key: key.into(),
                instance_id,
                native_request_id,
            },
        ))
    }
}

fn stage_name(input: &PaperConfirmationInputV1) -> &'static str {
    use PaperConfirmationInputV1 as Input;
    match input {
        Input::Request { .. } => "request",
        Input::OverviewReplay { .. } => "overviewReplay",
        Input::OverviewCandidate { .. } => "overviewCandidate",
        Input::OverviewFields { .. } => "overviewFields",
        Input::OverviewField { .. } => "overviewField",
        Input::UndoBatchKey { .. } => "undoBatchKey",
        Input::UndoReplayKey { .. } => "undoReplayKey",
        Input::ResetField { .. } => "resetField",
        Input::ResetEligibility { .. } => "resetEligibility",
        Input::ResetState { .. } => "resetState",
        Input::Unit { .. } => "unit",
        Input::PaperReplay { .. } => "paperReplay",
        Input::PaperExists { .. } => "paperExists",
        Input::PaperTarget { .. } => "paperTarget",
        Input::MetadataSource { .. } => "metadataSource",
    }
}
