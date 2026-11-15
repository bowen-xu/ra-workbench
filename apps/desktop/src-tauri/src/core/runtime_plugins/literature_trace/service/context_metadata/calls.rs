//! Exact-input projection through the same request-file and host owner.
use super::*;
use crate::runtime_plugins::context_metadata::ContextMetadataCapture;
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    context_metadata::{ContextMetadataDecisionV1, ContextMetadataRequestV1},
    paper_context_wire::encode_payload,
};

impl LiteratureTraceService {
    pub(super) async fn project_context_metadata(
        &self,
        host: &mut NativeHost,
        capture: Arc<ContextMetadataCapture>,
    ) -> NativeResult<String> {
        let cbor_base64 = tokio::task::spawn_blocking(move || encode_payload(&capture.input))
            .await
            .map_err(|_| {
                Self::error(
                    "plugin_host_context_metadata_encode_failed",
                    "文献元数据输入编码未完成。",
                )
            })?
            .map_err(|_| {
                Self::error(
                    "plugin_host_context_metadata_encode_failed",
                    "文献元数据输入不符合纯调用契约。",
                )
            })?;
        let (input, expected) = tokio::task::spawn_blocking(move || {
            let request = ContextMetadataRequestV1 { cbor_base64 };
            (request.clone(), request)
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_context_metadata_encode_failed",
                "文献元数据输入未完成。",
            )
        })?;
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let before = host.request_identity();
        let output = tokio::select! { biased;
            _=&mut stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭文献来源读取。")),
            output=host.catalog_call(CatalogInput::ProjectContextMetadata { input: Box::new(input) }, Duration::from_secs(60)) => output,
        }
        .map_err(|mut error| {
            match host.request_identity() {
                Some((instance, request)) if before.as_ref() != Some(&(instance.clone(), request.clone())) => {
                    error.instance_id = Some(instance);
                    error.request_id = Some(request);
                }
                _ => error.request_id = None,
            }
            error
        })?;
        let CatalogOutput::ContextMetadata { outcome } = output else {
            return Err(metadata_mismatch(host));
        };
        let echoed = outcome.input;
        let matched = tokio::task::spawn_blocking(move || *echoed == expected)
            .await
            .map_err(|_| {
                host.error(
                    "plugin_host_context_metadata_match_failed",
                    "文献元数据结果核对未完成。",
                )
            })?;
        if !matched {
            return Err(metadata_mismatch(host));
        }
        match outcome.decision {
            ContextMetadataDecisionV1::Projected { body_json } => Ok(body_json),
            ContextMetadataDecisionV1::Rejected { .. } => Err(host.error(
                "plugin_host_context_metadata_rejected",
                "文献元数据基础值不符合原正文投影契约。",
            )),
        }
    }
}
fn metadata_mismatch(host: &NativeHost) -> Box<NativeHostError> {
    host.error(
        "plugin_host_response_mismatch",
        "文献元数据纯结果与本次输入或阶段不匹配。",
    )
}
