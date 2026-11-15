//! One finite paper trace exchange on the existing managed literature host.
use super::*;
use crate::{
    ai::structured_resource_validation::DecisionOrigin,
    runtime_plugins::literature_trace::data::{PreparedTrace, TraceResources},
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    literature_trace_protocol::TraceDecision,
    retrieval_trace::MarkSentCondition,
};

impl LiteratureTraceService {
    pub(super) async fn paper_trace_resources(&self, key: &str) -> NativeResult<TraceResources> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_paper_retrieval_trace(&catalog, &selected)?;
            TraceResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_resource_worker_unsettled",
                "论文检索资源检查未确认完成。",
            )
        })?
    }
    pub(super) async fn paper_trace_decision(
        &self,
        host: &mut NativeHost,
        prepared: Box<PreparedTrace>,
    ) -> NativeResult<(Box<PreparedTrace>, TraceDecision, DecisionOrigin)> {
        let (prepared, input, expected) = tokio::task::spawn_blocking(move || {
            let input = prepared.paper_input()?;
            let expected = input.clone();
            Ok::<_, crate::runtime_plugins::literature_trace::data::TraceDataError>((
                prepared, input, expected,
            ))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_source_worker_unsettled",
                "论文检索输入检查未确认完成。",
            )
        })?
        .map_err(|_| {
            Self::error(
                "plugin_host_source_input_invalid",
                "论文检索输入范围不匹配。",
            )
        })?;
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let before = host.request_identity();
        let output = tokio::select! {biased;
            _=&mut stopped=>Err(Self::error("plugin_host_stopping","应用正在关闭论文检索来源准备。")),
            output=host.catalog_call(CatalogInput::DecidePaperRetrievalTrace {input:Box::new(input)},Duration::from_secs(60))=>output,
        }.map_err(|mut error| {
            match host.request_identity() {
                Some((instance,request)) if before.as_ref()!=Some(&(instance.clone(),request.clone())) => {
                    error.instance_id=Some(instance);
                    error.request_id=Some(request);
                }
                _ => error.request_id=None,
            }
            error
        })?;
        let CatalogOutput::PaperRetrievalTrace { outcome } = output else {
            return Err(host.error("plugin_host_response_mismatch", "论文检索结果种类不匹配。"));
        };
        let research_workbench_native_sdk::paper_retrieval::PaperRetrievalOutcomeV1 {
            input: echoed,
            decision,
        } = *outcome;
        let matches = tokio::task::spawn_blocking(move || *echoed == expected)
            .await
            .map_err(|_| {
                host.error(
                    "plugin_host_source_worker_unsettled",
                    "论文检索输入回显检查未确认完成。",
                )
            })?;
        if !matches
            || matches!(&decision,TraceDecision::Ready {mark_sent} if *mark_sent!=MarkSentCondition::ORIGINAL)
        {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "论文检索结果与原输入或消费后置条件不匹配。",
            ));
        }
        let Some((instance_id, native_request_id)) = host.request_identity() else {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "论文检索结果缺少本次纯决定身份。",
            ));
        };
        Ok((
            prepared,
            decision,
            DecisionOrigin {
                package_key: host.package_key().to_owned(),
                instance_id,
                native_request_id,
            },
        ))
    }
}
