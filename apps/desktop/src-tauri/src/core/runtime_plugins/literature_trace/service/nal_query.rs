//! Query reads and already accepted CAS writes share the original host owner.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_nal_query::{self, NalQueryResources, QueryOutcome},
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    nal_query::{Content, PrepareOutcome, ProjectionOutcome, Rejection, Stored},
};

mod errors;

#[cfg(test)]
mod tests;

impl LiteratureTraceService {
    pub(crate) async fn read_nal_queries(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
    ) -> NativeResult<Stored> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let result =
            tokio::spawn(async move { owner.read_queries_owned(database, &selected).await })
                .await
                .map_err(|_| Self::error("storage_error", "查询集合读取任务未能结算。"))
                .and_then(|result| result);
        result.map_err(|error| error.package(&key))
    }

    pub(crate) async fn save_nal_queries(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        value: Content,
        expected_revision: i64,
    ) -> NativeResult<Stored> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        // The Core task retains the host mutex through the original SQL work.
        // A caller dropping its wait cannot cancel that accepted transaction.
        let result = tokio::spawn(async move {
            owner
                .save_queries_owned(database, &selected, value, expected_revision)
                .await
        })
        .await
        .map_err(|_| errors::unknown("查询集合保存任务未能结算，禁止自动重试。"))
        .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            error.query_expected_revision = Some(expected_revision);
            if error.query_outcome.is_none() {
                error.query_outcome = Some(QueryOutcome::NotStarted);
            }
            error
        })
    }

    async fn query_resources(&self, key: &str) -> NativeResult<NalQueryResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_nal_queries(&catalog, &selected)?;
            NalQueryResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "查询集合资源核对任务未完成。"))?
    }

    async fn read_queries_owned(
        &self,
        database: SharedDatabase,
        key: &str,
    ) -> NativeResult<Stored> {
        let resources = self.query_resources(key).await?;
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
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                let row = tokio::task::spawn_blocking(move || {
                    let guard = database.lock().map_err(errors::owner)?;
                    resources.read_raw(&guard).map_err(errors::cause)
                }).await.map_err(|_| errors::bind(host, Self::error("storage_error", "查询集合读取任务未完成。")))?
                    .map_err(|error| errors::bind(host, error))?;
                let result = tokio::select! {
                    biased;
                    _ = &mut stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭查询集合读取。")),
                    result = host.catalog_call(CatalogInput::ReadNalQueries { row: row.clone() }, Duration::from_secs(60)) => result,
                }?;
                match result {
                    CatalogOutput::NalQueries { outcome: ProjectionOutcome::Success { row: returned } } if row == returned => {
                        tokio::task::spawn_blocking(move || returned.decode()).await
                            .map_err(|_| errors::bind(host, Self::error("storage_error", "查询集合解码任务未完成。")))?
                            .map_err(|_| host.error("plugin_host_response_mismatch", "查询集合投影未满足原结构契约。"))
                    }
                    CatalogOutput::NalQueries { outcome: ProjectionOutcome::Rejected { reason: Rejection::InvalidStructure } } =>
                        Err(errors::bind(host, errors::rejected(Rejection::InvalidStructure))),
                    _ => Err(host.error("plugin_host_response_mismatch", "查询集合投影与本次原始读取不匹配。")),
                }
            }.await;
            self.finish_domain(&mut slot, result).await
        }
    }

    async fn save_queries_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        value: Content,
        expected_revision: i64,
    ) -> NativeResult<Stored> {
        let resources = self.query_resources(key).await?;
        #[cfg(not(unix))]
        {
            let _ = (database, resources, value, expected_revision);
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
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                let owner_probe = Arc::clone(&database);
                let content = tokio::task::spawn_blocking(move || {
                    // Original connection availability precedes validate. This
                    // is not a BEGIN probe: query validation precedes BEGIN.
                    drop(owner_probe.lock().map_err(errors::owner)?);
                    research_workbench_native_sdk::nal_query::SavePlan::encode(&value)
                        .map_err(|_| errors::rejected(Rejection::InvalidStructure))
                }).await.map_err(|_| errors::bind(host, Self::error("storage_error", "查询集合准备任务未完成。")))?
                    .map_err(|error| errors::bind(host, error))?;
                let prepared = tokio::select! {
                    biased;
                    _ = &mut stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭查询集合保存。")),
                    result = host.catalog_call(CatalogInput::PrepareNalQuerySave { content }, Duration::from_secs(60)) => result,
                }?;
                let plan = match prepared {
                    CatalogOutput::NalQuerySavePlan { outcome: PrepareOutcome::Planned { plan } } => plan,
                    CatalogOutput::NalQuerySavePlan { outcome: PrepareOutcome::Rejected { reason } } =>
                        return Err(errors::bind(host, errors::rejected(reason))),
                    _ => return Err(host.error("plugin_host_response_mismatch", "查询集合保存准备阶段不匹配。")),
                };
                self.available()?;
                // Do not select shutdown against this accepted SQL task.
                let stored = tokio::task::spawn_blocking(move || {
                    let mut guard = database.lock().map_err(errors::owner)?;
                    literature_nal_query::save(&mut guard, &resources, &plan, expected_revision).map_err(errors::failure)
                }).await.map_err(|_| errors::bind(host, errors::unknown("查询集合写入任务未能结算，禁止自动重试。")))?
                    .map_err(|error| errors::bind(host, error))?;
                if self.stopping.load(Ordering::Acquire) {
                    let mut error = host.error("plugin_host_stopping", "查询集合已提交，应用正在关闭。" );
                    error.query_outcome = Some(QueryOutcome::Committed { revision: stored.revision });
                    return Err(error);
                }
                Ok(stored)
            }.await;
            self.finish_domain(&mut slot, result).await
        }
    }
}
