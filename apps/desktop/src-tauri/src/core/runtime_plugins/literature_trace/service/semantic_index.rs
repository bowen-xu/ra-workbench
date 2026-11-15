//! Finite original index rules, using the same managed literature host.
use super::*;
use crate::ai::structured_resource_validation::DecisionOrigin;
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    paper_context_wire::{decode_payload, encode_payload},
    semantic_index::rules::{IndexDecisionV1, IndexInputV1, IndexWireInputV1},
};
impl LiteratureTraceService {
    pub(crate) async fn semantic_index_rule(
        &self,
        key: &str,
        input: IndexInputV1,
    ) -> NativeResult<(IndexDecisionV1, DecisionOrigin)> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let (encoded, expected_input) = tokio::task::spawn_blocking(move || {
            package::accept_semantic_index(&catalog, &selected)?;
            let encoded = encode_payload(&input).map_err(|_| {
                Self::error("plugin_host_source_input_invalid", "索引规则输入无法编码。")
            })?;
            Ok::<_, Box<NativeHostError>>((encoded, input))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_source_worker_unsettled",
                "索引输入准备未确认完成。",
            )
        })??;
        #[cfg(not(unix))]
        {
            let _ = (encoded, expected_input);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            self.available()?;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                let stopped = self.stopped.notified();
                tokio::pin!(stopped);
                stopped.as_mut().enable();
                self.available()?;
                let before = host.request_identity();
                let expected = encoded.clone();
                let input = CatalogInput::PrepareSemanticIndex {
                    input: Box::new(IndexWireInputV1 {
                        cbor_base64: encoded,
                    }),
                };
                let output = tokio::select! {
                    biased;
                    _ = &mut stopped => Err(Self::error(
                        "plugin_host_stopping", "应用正在关闭概念索引规则准备。",
                    )),
                    output = host.catalog_call(input, Duration::from_secs(60)) => output,
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
                let CatalogOutput::SemanticIndex { outcome } = output else {
                    return Err(
                        host.error("plugin_host_response_mismatch", "索引规则结果种类不匹配。")
                    );
                };
                let decision = tokio::task::spawn_blocking(move || {
                    if outcome.input.cbor_base64 != expected {
                        return None;
                    }
                    let decision: IndexDecisionV1 =
                        decode_payload(&outcome.decision_cbor_base64).ok()?;
                    decision.matches_input(&expected_input).then_some(decision)
                })
                .await
                .map_err(|_| {
                    host.error(
                        "plugin_host_source_worker_unsettled",
                        "索引规则结果检查未确认完成。",
                    )
                })?
                .ok_or_else(|| {
                    host.error(
                        "plugin_host_response_mismatch",
                        "索引规则结果与原输入不匹配。",
                    )
                })?;
                let Some((instance_id, native_request_id)) = host.request_identity() else {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "索引规则结果缺少本次身份。",
                    ));
                };
                Ok((
                    decision,
                    DecisionOrigin {
                        package_key: key.into(),
                        instance_id,
                        native_request_id,
                    },
                ))
            }
            .await;
            self.finish_domain(&mut slot, result).await
        }
    }
}
