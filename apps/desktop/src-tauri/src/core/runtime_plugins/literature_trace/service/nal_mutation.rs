//! The same owned host prepares mutations; Core alone owns their transaction.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        literature_nal_bootstrap as bootstrap,
        literature_nal_mutation::{self as mutation_io, MutationOutcome, MutationResources},
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    nal_bootstrap::BootstrapInputV1,
    nal_mutation::{NalMutation, NalMutationPrepareInput},
    nal_state::NalState,
};

mod errors;

impl LiteratureTraceService {
    pub(crate) async fn mutate_nal_network(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        mutation: NalMutation,
        expected_revision: i64,
    ) -> NativeResult<NalState> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let result = tokio::spawn(async move {
            owner
                .mutation_owned(database, &selected, mutation, expected_revision)
                .await
        })
        .await
        .map_err(|_| errors::unknown("关系修改任务未能结算，禁止自动重试。"))
        .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.mutation_outcome.is_none() {
                error.mutation_outcome = Some(MutationOutcome::NotStarted);
            }
            error
        })
    }

    async fn mutation_resources(&self, key: &str) -> NativeResult<MutationResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_nal_mutation(&catalog, &selected)?;
            MutationResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "关系修改资源核对任务未完成。"))?
    }

    async fn mutation_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        mutation: NalMutation,
        expected_revision: i64,
    ) -> NativeResult<NalState> {
        let resources = self.mutation_resources(key).await?;
        #[cfg(not(unix))]
        {
            let _ = (database, resources, mutation, expected_revision);
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
            let mut request_before = None;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                request_before = host.request_identity();
                // No mutation rejection is consumed before startup/load/CAS.
                // Original DTO JSON is a string inside the transport envelope,
                // preserving its established term-depth decoding boundary.
                let mutation_json = serde_json::to_string(&mutation)
                    .map_err(|_| errors::bind(host, errors::codec()))?;
                let db = Arc::clone(&database);
                let source_resources = resources.bootstrap().clone();
                let previous = tokio::task::spawn_blocking(move || {
                    let mut guard = db.lock().map_err(errors::owner)?;
                    bootstrap::capture_source(&mut guard, &source_resources).map_err(errors::capture)
                }).await.map_err(|_| errors::bind(host, errors::unknown("关系修改准备读取未能结算，禁止自动重试。")))?
                    .map_err(|error| errors::bind(host, error))?;
                let source = Box::new(previous.source.clone());
                let output = tokio::select! {
                    biased;
                    _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭关系修改准备。")),
                    output=host.catalog_call(CatalogInput::PrepareNalBootstrapProbes { source },Duration::from_secs(60))=>output,
                }?;
                let CatalogOutput::NalBootstrapProbes { outcome: probes } = output else {
                    return Err(host.error("plugin_host_response_mismatch", "关系修改探针准备阶段不匹配。"));
                };
                let identities = super::nal_bootstrap::allocate_bootstrap_identities(&previous.source, &probes)
                    .map_err(|error| errors::bind(host, error))?;
                self.available().map_err(|error| errors::bind(host, error))?;
                let db = Arc::clone(&database);
                let probe_resources = resources.bootstrap().clone();
                let (captured, input) = tokio::task::spawn_blocking(move || {
                    let mut guard = db.lock().map_err(errors::owner)?;
                    let captured = bootstrap::capture_facts(&mut guard, &probe_resources, &previous, &probes, &identities)
                        .map_err(errors::capture)?;
                    let bootstrap = BootstrapInputV1 { source: captured.source.clone(), probes: *probes, facts: captured.facts.clone(), identities };
                    Ok::<_,Box<NativeHostError>>((captured, NalMutationPrepareInput { bootstrap, mutation_json }))
                }).await.map_err(|_| errors::bind(host, errors::unknown("关系修改探针读取未能结算，禁止自动重试。")))?
                    .map_err(|error| errors::bind(host,error))?;
                let wire_input = Box::new(input.clone());
                let output = tokio::select! {
                    biased;
                    _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭关系修改准备。")),
                    output=host.catalog_call(CatalogInput::PrepareNalMutation { input: wire_input },Duration::from_secs(60))=>output,
                }?;
                let CatalogOutput::NalMutationPreparation { outcome: plan } = output else {
                    return Err(host.error("plugin_host_response_mismatch", "关系修改提交准备阶段不匹配。"));
                };
                self.available().map_err(|error| errors::bind(host,error))?;
                // No cancellation select or host wait crosses accepted T1.
                // This owned task and the same host mutex survive a dropped
                // caller; shutdown waits for the original owner to settle.
                let state = tokio::task::spawn_blocking(move || {
                    let mut guard = database.lock().map_err(errors::owner)?;
                    mutation_io::apply(&mut guard, &resources, &captured, &input, &plan, &mutation, expected_revision)
                        .map_err(errors::failure)
                }).await.map_err(|_| errors::bind(host, errors::unknown("关系修改提交任务未能结算，禁止自动重试。")))?
                    .map_err(|error| errors::bind(host,error))?;
                if self.stopping.load(Ordering::Acquire) {
                    let mut error = host.error("plugin_host_stopping", "关系修改已提交，应用正在关闭。");
                    error.mutation_outcome = Some(MutationOutcome::Committed { revision: state.revision });
                    return Err(error);
                }
                Ok(state)
            }.await;
            // Retirement may attach the host's last exchange again. Filter
            // only after it settles: an initialize/previous-operation request
            // is not this mutation's native request. Actual new exchanges keep
            // their identities; the instance and cleanup diagnostics survive.
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|error| errors::scope_request(error, request_before.as_ref()))
        }
    }
}

#[cfg(test)]
mod tests;
