//! Owned coordination only. Native computes the fixed domain preparation;
//! Core retains the original database, transaction and shutdown ownership.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_nal_bootstrap::{self, BootstrapOutcome, BootstrapResources},
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    nal_bootstrap::BootstrapInputV1,
    nal_state::NalState,
};

mod errors;
mod identities;
/// Shared physical identity allocation for the two existing startup consumers.
pub(super) fn allocate_bootstrap_identities(
    source: &research_workbench_native_sdk::nal_bootstrap::BootstrapSourceV1,
    probes: &research_workbench_native_sdk::nal_bootstrap::BootstrapProbePreparationV1,
) -> NativeResult<research_workbench_native_sdk::nal_bootstrap::BootstrapIdentitiesV1> {
    identities::allocate(source, probes)
}
#[cfg(test)]
mod tests;

impl LiteratureTraceService {
    pub(crate) async fn read_nal_bootstrap(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
    ) -> NativeResult<NalState> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let result = tokio::spawn(async move { owner.bootstrap_owned(database, &selected).await })
            .await
            .map_err(|_| errors::unknown("关系网络任务未能结算，禁止自动重试。"))
            .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.bootstrap_outcome.is_none() {
                error.bootstrap_outcome = Some(BootstrapOutcome::NotStarted);
            }
            error
        })
    }

    async fn bootstrap_resources(&self, key: &str) -> NativeResult<BootstrapResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_nal_bootstrap(&catalog, &selected)?;
            BootstrapResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "关系网络资源核对任务未完成。"))?
    }

    async fn bootstrap_owned(&self, database: SharedDatabase, key: &str) -> NativeResult<NalState> {
        let resources = self.bootstrap_resources(key).await?;
        #[cfg(not(unix))]
        {
            let _ = (database, resources);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available()?;
            let result=async{
                let host=self.ensure_host(&mut slot,key).await?;
                let db=Arc::clone(&database);let source_resources=resources.clone();
                let previous=tokio::task::spawn_blocking(move||{
                    let mut guard=db.lock().map_err(errors::owner)?;
                    literature_nal_bootstrap::capture_source(&mut guard,&source_resources).map_err(errors::failure)
                }).await.map_err(|_|errors::bind(host,errors::unknown("关系网络准备读取未能结算，禁止自动重试。")))?
                    .map_err(|error|errors::bind(host,error))?;
                let source=Box::new(previous.source.clone());
                let output=tokio::select!{
                    biased;
                    _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭关系网络准备。")),
                    output=host.catalog_call(CatalogInput::PrepareNalBootstrapProbes{source},Duration::from_secs(60))=>output,
                }?;
                let CatalogOutput::NalBootstrapProbes{outcome:probes}=output else{
                    return Err(host.error("plugin_host_response_mismatch","关系网络探针准备阶段不匹配。"));
                };
                let identities=identities::allocate(&previous.source,&probes)
                    .map_err(|error|errors::bind(host,error))?;
                self.available().map_err(|error|errors::bind(host,error))?;
                let db=Arc::clone(&database);let probe_resources=resources.clone();
                let (captured,input)=tokio::task::spawn_blocking(move||{
                    let mut guard=db.lock().map_err(errors::owner)?;
                    let captured=literature_nal_bootstrap::capture_facts(&mut guard,&probe_resources,&previous,&probes,&identities)
                        .map_err(errors::failure)?;
                    let input=BootstrapInputV1{source:captured.source.clone(),probes:*probes,facts:captured.facts.clone(),identities};
                    Ok::<_,Box<NativeHostError>>((captured,input))
                }).await.map_err(|_|errors::bind(host,errors::unknown("关系网络探针读取未能结算，禁止自动重试。")))?
                    .map_err(|error|errors::bind(host,error))?;
                let wire_input=Box::new(input.clone());
                let output=tokio::select!{
                    biased;
                    _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭关系网络准备。")),
                    output=host.catalog_call(CatalogInput::PrepareNalBootstrap{input:wire_input},Duration::from_secs(60))=>output,
                }?;
                let CatalogOutput::NalBootstrapPlan{outcome:plan}=output else{
                    return Err(host.error("plugin_host_response_mismatch","关系网络提交准备阶段不匹配。"));
                };
                self.available().map_err(|error|errors::bind(host,error))?;
                // Once accepted, this SQL worker is settled before shutdown can
                // acquire the same host mutex. No native await enters its guard.
                let state=tokio::task::spawn_blocking(move||{
                    let mut guard=database.lock().map_err(errors::owner)?;
                    literature_nal_bootstrap::apply(&mut guard,&resources,&captured,&input,&plan).map_err(errors::failure)
                }).await.map_err(|_|errors::bind(host,errors::unknown("关系网络提交任务未能结算，禁止自动重试。")))?
                    .map_err(|error|errors::bind(host,error))?;
                if self.stopping.load(Ordering::Acquire){
                    let mut error=host.error("plugin_host_stopping","关系网络已提交，应用正在关闭。");
                    error.bootstrap_outcome=Some(BootstrapOutcome::Committed{revision:state.revision});
                    return Err(error);
                }
                Ok(state)
            }.await;
            self.finish_domain(&mut slot, result).await
        }
    }
}
