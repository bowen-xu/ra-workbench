//! Three request-bound pure calls over the original host and input-file owner.
use super::*;
use crate::ai::structured_resource_validation::{DecisionOrigin, Failure};
use crate::runtime_plugins::literature_context::BuildCapture;
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    paper_context::{
        PreparedPaperContextV1, SnapshotDigestInputV1, SnapshotProjectionInputV1,
        SnapshotProjectionV1,
    },
    paper_context_wire::{
        PaperContextCodecError, PaperContextDecisionV1 as Decision, PaperContextInputV1 as Input,
        decode_payload, encode_payload,
    },
};
use serde::de::DeserializeOwned;

impl LiteratureTraceService {
    pub(in crate::runtime_plugins::literature_trace::service) async fn prepare_context_stage(
        &self,
        host: &mut NativeHost,
        input: research_workbench_native_sdk::paper_context_prepare::PaperPreparationInputV1,
    ) -> NativeResult<
        research_workbench_native_sdk::paper_context_prepare::PaperPreparationDecisionV1,
    > {
        use research_workbench_native_sdk::paper_context_prepare::{
            PaperPreparationDecisionV1 as Prepared, PaperPreparationInputV1 as Stage,
        };
        let kind = match &input {
            Stage::AnalysisSources { .. } => 4,
            Stage::Session { .. } => 0,
            Stage::SnapshotReady { .. } => 1,
            Stage::ImageIdentity { .. } => 2,
            Stage::ImageSet { .. } => 3,
        };
        let cbor_base64 = encode_context(host, move || encode_payload(&input)).await?;
        let Decision::Prepared {
            decision_cbor_base64,
        } = self
            .context_call(host, Input::Prepare { cbor_base64 })
            .await?
        else {
            return Err(context_mismatch(host));
        };
        let decision = decode_context(host, decision_cbor_base64).await?;
        match (&decision, kind) {
            (Prepared::Session { .. }, 0)
            | (Prepared::Ready { .. }, 1)
            | (Prepared::Validated {}, 2..=4)
            | (Prepared::Rejected { .. }, 0 | 2..=4) => Ok(decision),
            _ => Err(context_mismatch(host)),
        }
    }

    pub(in crate::runtime_plugins::literature_trace::service) async fn preview_context_stage(
        &self,
        host: &mut NativeHost,
        input: research_workbench_native_sdk::paper_context_preview::PaperContextPreviewInputV1,
    ) -> NativeResult<
        research_workbench_native_sdk::paper_context_preview::PaperContextPreviewDecisionV1,
    > {
        use research_workbench_native_sdk::paper_context_preview::{
            PaperContextPreviewDecisionV1 as PreviewDecision, PaperContextPreviewInputV1 as Stage,
        };
        let snapshot = matches!(&input, Stage::Snapshot { .. });
        let cbor_base64 = encode_context(host, move || encode_payload(&input)).await?;
        let Decision::Previewed {
            decision_cbor_base64,
        } = self
            .context_call(host, Input::Preview { cbor_base64 })
            .await?
        else {
            return Err(context_mismatch(host));
        };
        let decision = decode_context(host, decision_cbor_base64).await?;
        match &decision {
            PreviewDecision::Validated {} if !snapshot => Ok(decision),
            PreviewDecision::Previewed { .. } if snapshot => Ok(decision),
            PreviewDecision::Rejected { .. } => Ok(decision),
            _ => Err(context_mismatch(host)),
        }
    }
    async fn context_call(&self, host: &mut NativeHost, input: Input) -> NativeResult<Decision> {
        let kind = match &input {
            Input::Build { .. } => 0,
            Input::Project { .. } => 1,
            Input::Digest { .. } => 2,
            Input::Preview { .. } => 3,
            Input::Prepare { .. } => 4,
        };
        // Payloads can contain a complete bounded parse. Copy and compare them
        // off the executor, under the same awaited source-read owner.
        let (request, expected) = tokio::task::spawn_blocking(move || (input.clone(), input))
            .await
            .map_err(|_| before_exchange(context_worker_error(host)))?;
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let before = host.request_identity();
        let output = tokio::select! {biased;
            _=&mut stopped=>Err(Self::error("plugin_host_stopping","应用正在关闭论文来源准备。")),
            output=host.catalog_call(CatalogInput::PreparePaperContext{input:Box::new(request)},Duration::from_secs(60))=>output,
        }
        .map_err(|mut error| {
            match host.request_identity() {
                Some((instance, request)) if before.as_ref() != Some(&(instance.clone(), request.clone())) => {
                    // Cancellation can win after an actual exchange started.
                    error.instance_id = Some(instance);
                    error.request_id = Some(request);
                }
                _ => error.request_id = None,
            }
            error
        })?;
        let CatalogOutput::PaperContext { outcome } = output else {
            return Err(context_mismatch(host));
        };
        let echoed = outcome.input;
        let matches = tokio::task::spawn_blocking(move || *echoed == expected)
            .await
            .map_err(|_| context_worker_error(host))?;
        if !matches {
            return Err(context_mismatch(host));
        }
        match (kind, &outcome.decision) {
            (0, Decision::Built { .. })
            | (1, Decision::Projected { .. })
            | (2, Decision::Digested { .. })
            | (3, Decision::Previewed { .. })
            | (4, Decision::Prepared { .. }) => Ok(outcome.decision),
            (1, Decision::Rejected { message })
                if message == "只有完成 Docling 解析的附件才能冻结论文上下文" =>
            {
                Ok(outcome.decision)
            }
            (_, Decision::Unavailable {}) => Err(host.error(
                "plugin_host_paper_context_unavailable",
                "论文上下文的纯准备未完成。",
            )),
            _ => Err(context_mismatch(host)),
        }
    }
    pub(in crate::runtime_plugins::literature_trace::service) async fn build_context(
        &self,
        host: &mut NativeHost,
        input: Arc<BuildCapture>,
    ) -> NativeResult<PreparedPaperContextV1> {
        let cbor_base64 = encode_context(host, move || encode_payload(&input.input)).await?;
        let Decision::Built {
            prepared_cbor_base64,
        } = self
            .context_call(host, Input::Build { cbor_base64 })
            .await?
        else {
            return Err(context_mismatch(host));
        };
        decode_context(host, prepared_cbor_base64).await
    }
    pub(in crate::runtime_plugins::literature_trace::service) async fn project_context(
        &self,
        host: &mut NativeHost,
        key: &str,
        input: Arc<SnapshotProjectionInputV1>,
    ) -> NativeResult<Result<SnapshotProjectionV1, Failure<crate::ai::error::AiError>>> {
        let cbor_base64 = encode_context(host, move || encode_payload(input.as_ref())).await?;
        match self
            .context_call(host, Input::Project { cbor_base64 })
            .await?
        {
            Decision::Projected {
                projection_cbor_base64,
            } => Ok(Ok(decode_context(host, projection_cbor_base64).await?)),
            Decision::Rejected { message } => {
                let Some((instance_id, native_request_id)) = host.request_identity() else {
                    return Err(context_mismatch(host));
                };
                Ok(Err(Failure {
                    error: crate::ai::error::AiError::InvalidInput(message),
                    cause: crate::ai::structured_resource_validation::FailureCause::NativeRejection(
                        Box::new(DecisionOrigin {
                            package_key: key.into(),
                            instance_id,
                            native_request_id,
                        }),
                    ),
                }))
            }
            _ => Err(context_mismatch(host)),
        }
    }
    pub(in crate::runtime_plugins::literature_trace::service) async fn digest_context(
        &self,
        host: &mut NativeHost,
        input: SnapshotDigestInputV1,
    ) -> NativeResult<String> {
        let cbor_base64 = encode_context(host, move || encode_payload(&input)).await?;
        let Decision::Digested { context_hash } = self
            .context_call(host, Input::Digest { cbor_base64 })
            .await?
        else {
            return Err(context_mismatch(host));
        };
        if context_hash.len() != 64
            || !context_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(context_mismatch(host));
        }
        Ok(context_hash)
    }
}
async fn encode_context(
    host: &NativeHost,
    encode: impl FnOnce() -> Result<String, PaperContextCodecError> + Send + 'static,
) -> NativeResult<String> {
    tokio::task::spawn_blocking(encode)
        .await
        .map_err(|_| before_exchange(context_worker_error(host)))?
        .map_err(|error| before_exchange(context_codec_error(host, error)))
}

fn before_exchange(mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    // No request for this stage exists yet; the previous successful pure call
    // must not be presented as the identity of a local encoding failure.
    error.request_id = None;
    error
}

async fn decode_context<T: DeserializeOwned + Send + 'static>(
    host: &NativeHost,
    payload: String,
) -> NativeResult<T> {
    tokio::task::spawn_blocking(move || decode_payload(&payload))
        .await
        .map_err(|_| context_worker_error(host))?
        .map_err(|error| context_codec_error(host, error))
}

fn context_worker_error(host: &NativeHost) -> Box<NativeHostError> {
    host.error(
        "plugin_host_paper_context_worker_unsettled",
        "论文来源读取未确认结算；请核对原调用与快照，不要自动重复写入。",
    )
}
fn context_mismatch(host: &NativeHost) -> Box<NativeHostError> {
    host.error(
        "plugin_host_response_mismatch",
        "论文上下文纯结果与本次阶段或输入不匹配。",
    )
}

fn context_codec_error(host: &NativeHost, error: PaperContextCodecError) -> Box<NativeHostError> {
    host.error(
        "plugin_host_paper_context_payload_invalid",
        format!(
            "paper-context payload invalid (kind={:?}, offset={:?})",
            error.kind, error.offset
        ),
    )
}
