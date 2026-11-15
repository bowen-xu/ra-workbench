//! Owned original history reads. Database capture finishes before the Native wait.
use super::*;
use crate::{
    ai::structured_resource_validation::DecisionOrigin,
    runtime_plugins::literature_analysis::{capture, failure::Error, resources::HistoryResources},
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_history::{
        HistoryInputV1, HistoryRequestV1, HistoryResultV1, HistoryWireInputV1,
        contracts::PaperAnalysisWorkflow,
    },
    catalog::{CatalogInput, CatalogOutput},
    paper_context_wire::{decode_payload, encode_payload},
};
use serde_json::Value;

pub(super) struct ObservedHistory {
    pub workflow: Option<PaperAnalysisWorkflow>,
    pub input: HistoryInputV1,
    pub origin: DecisionOrigin,
}

impl LiteratureTraceService {
    pub(crate) async fn read_analysis_history(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: HistoryRequestV1,
    ) -> Result<Option<PaperAnalysisWorkflow>, Value> {
        let selected = key.clone();
        self.analysis_history_result(store, key, request.clone())
            .await
            .map(|observed| observed.workflow)
            .map_err(|error| error.into_wire(&request, Some(&selected)))
    }

    pub(super) async fn read_analysis_history_observed_ai(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: HistoryRequestV1,
    ) -> Result<ObservedHistory, crate::ai::error::AiError> {
        let selected = key.clone();
        self.analysis_history_result(store, key, request)
            .await
            .map_err(|error| error.into_ai(&selected))
    }

    async fn analysis_history_result(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: HistoryRequestV1,
    ) -> Result<ObservedHistory, Error> {
        let owner = Arc::clone(self);
        tokio::spawn(async move { owner.read_analysis_history_owned(store, key, request).await })
            .await
            .map_err(|_| Error::unsettled())?
    }

    async fn read_analysis_history_owned(
        &self,
        store: AppStore,
        key: String,
        request: HistoryRequestV1,
    ) -> Result<ObservedHistory, Error> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        self.available()?;
        self.capture_project_analysis_history_observed(store, key, request)
            .await
    }

    pub(super) async fn capture_project_analysis_history(
        &self,
        store: AppStore,
        key: String,
        request: HistoryRequestV1,
    ) -> Result<Option<PaperAnalysisWorkflow>, Error> {
        self.capture_project_analysis_history_observed(store, key, request)
            .await
            .map(|observed| observed.workflow)
    }

    pub(super) async fn capture_project_analysis_history_observed(
        &self,
        store: AppStore,
        key: String,
        request: HistoryRequestV1,
    ) -> Result<ObservedHistory, Error> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let input = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_history(&catalog, &selected)?;
            let resources = HistoryResources::load(&catalog, &package)
                .map_err(|error| history_resource_error(error, &selected))?;
            let workflow = {
                let db = store.connection()?;
                capture::capture(&db, &request, &resources.statements())?
            };
            let input = HistoryInputV1 { request, workflow };
            encode_payload(&input)
                .map(|encoded| {
                    let expected = encoded.clone();
                    (encoded, expected)
                })
                .map_err(|_| {
                    Error::technical(
                        "plugin_host_source_input_invalid",
                        "论文分析历史输入无法编码。",
                    )
                })
        })
        .await
        .map_err(|_| Error::unsettled())??;
        self.available()?;
        self.project_analysis_history(&key, input).await
    }

    #[cfg(not(unix))]
    async fn project_analysis_history(
        &self,
        _key: &str,
        _input: (String, String),
    ) -> Result<ObservedHistory, Error> {
        Err(Error::technical(
            "plugin_host_platform_unsupported",
            "此平台尚未提供文献宿主。",
        ))
    }

    #[cfg(unix)]
    async fn project_analysis_history(
        &self,
        key: &str,
        input: (String, String),
    ) -> Result<ObservedHistory, Error> {
        let mut slot = self.host.lock().await;
        self.available()?;
        let result = async {
            let host = self.ensure_host(&mut slot, key).await?;
            self.analysis_history_stage(host, key, input).await
        }
        .await;
        let ((result, input), origin) = self.finish_domain(&mut slot, result).await?;
        Self::observed_analysis_history(result, input, origin)
    }
    #[cfg(unix)]
    pub(super) async fn analysis_history_stage(
        &self,
        host: &mut NativeHost,
        key: &str,
        input: (String, String),
    ) -> NativeResult<((HistoryResultV1, HistoryInputV1), DecisionOrigin)> {
        let (input, expected) = input;
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let before = host.request_identity();
        let request = CatalogInput::ProjectAnalysisHistory {
            input: Box::new(HistoryWireInputV1 { cbor_base64: input }),
        };
        let output = tokio::select! {
            biased;
            _ = &mut stopped => Err(Self::error(
                "plugin_host_stopping",
                "应用正在关闭论文分析历史读取。",
            )),
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
        let CatalogOutput::AnalysisHistory { outcome } = output else {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "论文分析历史结果种类不匹配。",
            ));
        };
        let decoded = tokio::task::spawn_blocking(move || {
            if outcome.input.cbor_base64 != expected {
                return Err(());
            }
            let result =
                decode_payload::<HistoryResultV1>(&outcome.result_cbor_base64).map_err(|_| ())?;
            let input: HistoryInputV1 = decode_payload(&expected).map_err(|_| ())?;
            if !input.matches_result(&result) {
                return Err(());
            }
            Ok((result, input))
        })
        .await
        .map_err(|_| {
            host.error(
                "plugin_host_source_worker_unsettled",
                "论文分析历史结果解码未确认完成。",
            )
        })?
        .map_err(|_| {
            host.error(
                "plugin_host_response_mismatch",
                "论文分析历史结果与本次输入不匹配。",
            )
        })?;
        let Some((instance_id, native_request_id)) = host.request_identity() else {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "论文分析历史结果缺少本次身份。",
            ));
        };
        Ok((
            decoded,
            DecisionOrigin {
                package_key: key.into(),
                instance_id,
                native_request_id,
            },
        ))
    }
    pub(super) fn observed_analysis_history(
        result: HistoryResultV1,
        input: HistoryInputV1,
        origin: DecisionOrigin,
    ) -> Result<ObservedHistory, Error> {
        match result {
            Ok(workflow) => Ok(ObservedHistory {
                workflow,
                input,
                origin,
            }),
            Err(error) => Err(Error::Rejected {
                message: error.to_string(),
                origin,
            }),
        }
    }
}

fn history_resource_error(
    error: crate::runtime_plugins::RuntimePluginError,
    package: &str,
) -> Box<NativeHostError> {
    let resource = match error.resource.as_deref() {
        Some("data/analysis-history-workflow.v1.sql") => Some("workflow"),
        Some("data/analysis-history-latest.v1.sql") => Some("latest"),
        Some("data/analysis-history-turns.v1.sql") => Some("turns"),
        Some("data/analysis-history-attempts.v1.sql") => Some("attempts"),
        _ => None,
    };
    let mut native = LiteratureTraceService::error(error.code, error.message).package(package);
    if let Some(resource) = resource {
        native.data_error = Some(Box::new(
            crate::data_interface::plugin_resources::PluginDataError {
                code: "plugin_data_resource_unavailable",
                message: "论文分析历史固定读取资源不可用。",
                package_key: package.into(),
                resource_id: format!("literature.analysis-history.{resource}"),
                resource_version: 1,
                phase: "resource",
                safe_cause: error.code.into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            },
        ));
    }
    native
}
