//! One original capture owner spans Native preparation and final Core settlement.
use super::*;
use crate::runtime_plugins::RuntimePluginError;
use crate::{
    ai::structured_resource_validation::DecisionOrigin,
    runtime_plugins::{
        literature_attachments::AttachmentResources,
        literature_browser_capture::{self, CaptureFailure, CaptureResources, CaptureStorage},
        literature_catalog::save::SaveResources,
    },
    storage::AppStore,
};
use hyper::body::Bytes;
use research_workbench_native_sdk::{
    browser_capture::{BrowserCaptureInputV1, BrowserCapturePreparationV1},
    catalog::{CatalogInput, CatalogOutput},
};
use serde_json::Value;

impl LiteratureTraceService {
    pub(crate) fn browser_capture_declared(&self, key: &str) -> NativeResult<()> {
        package::accept_browser_capture(&self.catalog, key).map(|_| ())
    }
    pub(crate) async fn browser_capture(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        scope: String,
        input: BrowserCaptureInputV1,
        bytes: Bytes,
    ) -> NativeResult<Value> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move {
            owner.available()?;
            let _lifetime = owner.doi_operations.read().await;
            owner.available()?;
            let resources = owner.browser_capture_storage(&key).await?;
            let (prepared, origin) = owner.prepare_browser_capture(&key, input).await?;
            owner.available()?;
            tokio::task::spawn_blocking(move || {
                literature_browser_capture::execute(&store, &resources, &scope, prepared, &bytes)
            })
            .await
            .map_err(|_| {
                capture_failure(
                    CaptureFailure::Technical("browser_capture_owner_unsettled".into()),
                    &key,
                    Some(&origin),
                )
            })?
            .map_err(|error| capture_failure(error, &key, Some(&origin)))
        })
        .await
        .map_err(|_| {
            capture_failure(
                CaptureFailure::Technical("browser_capture_owner_unsettled".into()),
                &selected,
                None,
            )
        })?
    }

    async fn browser_capture_storage(&self, key: &str) -> NativeResult<CaptureStorage> {
        let catalog = Arc::clone(&self.catalog);
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_browser_capture(&catalog, &key)?;
            let failure =
                |error: RuntimePluginError| Self::error(error.code, error.message).package(&key);
            Ok(CaptureStorage {
                sql: CaptureResources::load(&catalog, &package).map_err(failure)?,
                catalog: SaveResources::load(&catalog, &package).map_err(failure)?,
                attachments: AttachmentResources::load_attach(&catalog, &package)
                    .map_err(failure)?,
            })
        })
        .await
        .map_err(|_| Self::error("worker_unavailable", "浏览器采集资源读取线程不可用。"))?
    }

    #[cfg(not(unix))]
    async fn prepare_browser_capture(
        &self,
        _key: &str,
        _input: BrowserCaptureInputV1,
    ) -> NativeResult<(BrowserCapturePreparationV1, DecisionOrigin)> {
        Err(Self::error(
            "plugin_host_platform_unsupported",
            "此平台尚未提供文献宿主。",
        ))
    }

    #[cfg(unix)]
    async fn prepare_browser_capture(
        &self,
        key: &str,
        input: BrowserCaptureInputV1,
    ) -> NativeResult<(BrowserCapturePreparationV1, DecisionOrigin)> {
        self.available()?;
        let mut slot = self.host.lock().await;
        self.available()?;
        let result = async {
            let host = self.ensure_host(&mut slot, key).await?;
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available()?;
            let before = host.request_identity();
            let expected = input.clone();
            let output = tokio::select! {
                biased;
                _ = &mut stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭浏览器采集。")),
                output = host.catalog_call(CatalogInput::PrepareBrowserCapture { input: Box::new(input) }, Duration::from_secs(60)) => output,
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
            let CatalogOutput::BrowserCapture { input, preparation } = output else {
                return Err(host.error("plugin_host_response_mismatch", "浏览器采集结果种类不匹配。"));
            };
            if *input != expected {
                return Err(host.error("plugin_host_response_mismatch", "浏览器采集结果与本次输入不匹配。"));
            }
            let Some((instance_id, native_request_id)) = host.request_identity() else {
                return Err(host.error("plugin_host_response_mismatch", "浏览器采集结果缺少本次身份。"));
            };
            Ok((*preparation, DecisionOrigin { package_key: key.into(), instance_id, native_request_id }))
        }.await;
        self.finish_domain(&mut slot, result).await
    }
}

fn capture_failure(
    error: CaptureFailure,
    key: &str,
    origin: Option<&DecisionOrigin>,
) -> Box<NativeHostError> {
    let mut failure = error.into_native().package(key);
    if let Some(origin) = origin {
        failure.instance_id = Some(origin.instance_id.clone());
        failure.request_id = Some(origin.native_request_id.clone());
    }
    failure
}
