//! Finite retrieval calls reuse the existing host and operation lifetime.
use super::*;
use crate::{
    ai::{CancellationSignal, EmbeddingAccess, structured_resource_validation::DecisionOrigin},
    runtime_plugins::literature_retrieval::{self, resources::RetrievalResources},
    storage::AppStore,
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    concept_retrieval::{
        ConceptRetrievalDecisionV1, ConceptRetrievalInputV1, ConceptRetrievalPreview,
        ConceptRetrievalPreviewDraft, ConceptRetrievalWireInputV1,
    },
    paper_context_wire::{decode_payload, encode_payload},
};
use serde_json::Value;

impl LiteratureTraceService {
    pub(crate) async fn preview_concept_retrieval(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        embedding: EmbeddingAccess,
        draft: ConceptRetrievalPreviewDraft,
        signal: CancellationSignal,
    ) -> Result<ConceptRetrievalPreview, Value> {
        let _lifetime = self.doi_operations.read().await;
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        let work = literature_retrieval::execute::preview(
            self,
            store,
            &key,
            embedding,
            draft,
            signal.clone(),
        );
        tokio::pin!(work);
        tokio::select! {biased;
            result=&mut work=>result,
            _=&mut stopped=>{signal.cancel();work.await},
        }
    }
    pub(crate) async fn read_concept_retrieval_trace(
        &self,
        store: AppStore,
        key: String,
        trace: String,
    ) -> Result<ConceptRetrievalPreview, Value> {
        let _lifetime = self.doi_operations.read().await;
        let requested = trace.clone();
        let result = async {
            let resources = self.concept_retrieval_resources(&key).await?;
            tokio::task::spawn_blocking(move || {
                let db = store
                    .connection()
                    .map_err(crate::ai::knowledge_contracts::ConceptRetrievalError::from)?;
                literature_retrieval::trace_read::read(&db, &resources, &trace)
            })
            .await
            .map_err(|_| literature_retrieval::failure::Error::Unsettled)?
        }
        .await;
        result.map_err(|error: literature_retrieval::failure::Error| {
            let mut value = error.into_wire(
                None,
                Some(&requested),
                "source-read",
                crate::hbs_storage::boundary::HbsOutcome::NotStarted,
            );
            value["retrievalOperation"] = serde_json::json!("trace-read");
            value
        })
    }
    pub(crate) async fn concept_retrieval_resources(
        &self,
        key: &str,
    ) -> NativeResult<Arc<RetrievalResources>> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_concept_retrieval(&catalog, &selected)?;
            RetrievalResources::load(&catalog, &package)
                .map(Arc::new)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_resource_worker_unsettled",
                "检索资源检查未确认完成。",
            )
        })?
    }
    pub(crate) async fn concept_retrieval_rule(
        &self,
        key: &str,
        input: ConceptRetrievalInputV1,
    ) -> NativeResult<(ConceptRetrievalDecisionV1, DecisionOrigin)> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let encoded = tokio::task::spawn_blocking(move || {
            package::accept_concept_retrieval(&catalog, &selected)?;
            encode_payload(&input).map_err(|_| {
                Self::error("plugin_host_source_input_invalid", "检索规则输入无法编码。")
            })
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_source_worker_unsettled",
                "检索输入准备未确认完成。",
            )
        })??;
        #[cfg(not(unix))]
        {
            let _ = encoded;
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
                let input = CatalogInput::PrepareConceptRetrieval {
                    input: Box::new(ConceptRetrievalWireInputV1 {
                        cbor_base64: encoded,
                    }),
                };
                let output = tokio::select! {
                    biased;
                    _ = &mut stopped => Err(Self::error(
                        "plugin_host_stopping", "应用正在关闭概念检索规则准备。",
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
                let CatalogOutput::ConceptRetrieval { outcome } = output else {
                    return Err(
                        host.error("plugin_host_response_mismatch", "检索规则结果种类不匹配。")
                    );
                };
                let decision = tokio::task::spawn_blocking(move || {
                    if outcome.input.cbor_base64 != expected {
                        return None;
                    }
                    decode_payload(&outcome.decision_cbor_base64).ok()
                })
                .await
                .map_err(|_| {
                    host.error(
                        "plugin_host_source_worker_unsettled",
                        "检索规则结果检查未确认完成。",
                    )
                })?
                .ok_or_else(|| {
                    host.error(
                        "plugin_host_response_mismatch",
                        "检索规则结果与原输入不匹配。",
                    )
                })?;
                let Some((instance_id, native_request_id)) = host.request_identity() else {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "检索规则结果缺少本次身份。",
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
