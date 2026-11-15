//! Owned ensure coordination; the shared host and Core remain the only owners.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        literature_nal_annotation::{self as annotation, AnnotationOutcome, AnnotationResources},
        literature_nal_bootstrap as bootstrap,
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    nal_annotation::{NalAnnotation, NalAnnotationRequest},
    nal_annotation_prepare::{
        NalAnnotationIdentities, NalAnnotationPrepareInput, NalAnnotationValidation,
    },
    nal_bootstrap::BootstrapInputV1,
};

mod errors;
#[cfg(test)]
mod tests;

impl LiteratureTraceService {
    pub(crate) async fn ensure_nal_annotation(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: NalAnnotationRequest,
    ) -> NativeResult<NalAnnotation> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let result =
            tokio::spawn(async move { owner.annotation_owned(database, &selected, request).await })
                .await
                .map_err(|_| errors::unknown("笔记关联任务未能结算，禁止自动重试。"))
                .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.annotation_outcome.is_none() {
                error.annotation_outcome = Some(AnnotationOutcome::NotStarted);
            }
            error
        })
    }

    async fn annotation_resources(&self, key: &str) -> NativeResult<AnnotationResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_nal_annotation(&catalog, &selected)?;
            AnnotationResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "笔记关联资源核对任务未完成。"))?
    }

    async fn annotation_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: NalAnnotationRequest,
    ) -> NativeResult<NalAnnotation> {
        let resources = self.annotation_resources(key).await?;
        #[cfg(not(unix))]
        {
            let _ = (database, resources, request);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            #[cfg(test)]
            if let Some(observed) = &self.admission_observed {
                observed.notify_one();
            }
            let mut slot = self.host.lock().await;
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available()?;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                // Original validation precedes every database capture/BEGIN.
                let output = tokio::select! {
                    biased;
                    _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭笔记关联准备。")),
                    output=host.catalog_call(CatalogInput::ValidateNalAnnotation { request: request.clone() },Duration::from_secs(60))=>output,
                }?;
                match output {
                    CatalogOutput::NalAnnotationValidation { outcome: NalAnnotationValidation::Valid {} } => {}
                    CatalogOutput::NalAnnotationValidation { outcome: NalAnnotationValidation::InvalidRequest {} } => {
                        return Err(errors::bind(host, errors::invalid_request()));
                    }
                    _ => return Err(host.error("plugin_host_response_mismatch", "笔记关联预校验阶段不匹配。")),
                }
                self.available().map_err(|error| errors::bind(host, error))?;
                let db = Arc::clone(&database);
                let source_resources = resources.bootstrap().clone();
                let previous = tokio::task::spawn_blocking(move || {
                    let mut guard = db.lock().map_err(errors::owner)?;
                    bootstrap::capture_source(&mut guard, &source_resources).map_err(errors::capture)
                }).await
                    .map_err(|_| errors::bind(host, errors::unknown("笔记关联准备读取未能结算，禁止自动重试。")))?
                    .map_err(|error| errors::bind(host, error))?;
                let source = Box::new(previous.source.clone());
                let output = tokio::select! {
                    biased;
                    _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭笔记关联准备。")),
                    output=host.catalog_call(CatalogInput::PrepareNalBootstrapProbes { source },Duration::from_secs(60))=>output,
                }?;
                let CatalogOutput::NalBootstrapProbes { outcome: probes } = output else {
                    return Err(host.error("plugin_host_response_mismatch", "笔记关联探针准备阶段不匹配。"));
                };
                let identities = super::nal_bootstrap::allocate_bootstrap_identities(&previous.source, &probes)
                    .map_err(|error| errors::bind(host, error))?;
                let annotation_identities = NalAnnotationIdentities {
                    page_id: uuid::Uuid::new_v4().to_string(),
                    annotation_id: uuid::Uuid::new_v4().to_string(),
                };
                self.available().map_err(|error| errors::bind(host, error))?;
                let db = Arc::clone(&database);
                let probe_resources = resources.bootstrap().clone();
                let (captured, input) = tokio::task::spawn_blocking(move || {
                    let mut guard = db.lock().map_err(errors::owner)?;
                    let captured = bootstrap::capture_facts(&mut guard, &probe_resources, &previous, &probes, &identities)
                        .map_err(errors::capture)?;
                    let bootstrap = BootstrapInputV1 { source: captured.source.clone(), probes: *probes, facts: captured.facts.clone(), identities };
                    let input = NalAnnotationPrepareInput { bootstrap, request, identities: annotation_identities };
                    Ok::<_,Box<NativeHostError>>((captured,input))
                }).await
                    .map_err(|_| errors::bind(host, errors::unknown("笔记关联探针读取未能结算，禁止自动重试。")))?
                    .map_err(|error| errors::bind(host,error))?;
                let wire_input = Box::new(input.clone());
                let output = tokio::select! {
                    biased;
                    _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭笔记关联准备。")),
                    output=host.catalog_call(CatalogInput::PrepareNalAnnotation { input: wire_input },Duration::from_secs(60))=>output,
                }?;
                let CatalogOutput::NalAnnotationPreparation { outcome: plan } = output else {
                    return Err(host.error("plugin_host_response_mismatch", "笔记关联提交准备阶段不匹配。"));
                };
                self.available().map_err(|error| errors::bind(host,error))?;
                // The accepted Immediate SQL worker is owned until settled.
                // Shutdown waits on this same host mutex; no host await enters
                // the transaction or interrupts Core Note creation.
                let annotation = tokio::task::spawn_blocking(move || {
                    let mut guard = database.lock().map_err(errors::owner)?;
                    annotation::apply(&mut guard, &resources, &captured, &input, &plan).map_err(errors::failure)
                }).await
                    .map_err(|_| errors::bind(host, errors::unknown("笔记关联提交任务未能结算，禁止自动重试。")))?
                    .map_err(|error| errors::bind(host,error))?;
                if self.stopping.load(Ordering::Acquire) {
                    let mut error = host.error("plugin_host_stopping", "笔记关联已提交，应用正在关闭。");
                    error.annotation_outcome = Some(AnnotationOutcome::Committed { annotation: Box::new(annotation) });
                    return Err(error);
                }
                Ok(annotation)
            }.await;
            self.finish_domain(&mut slot, result).await
        }
    }
}
