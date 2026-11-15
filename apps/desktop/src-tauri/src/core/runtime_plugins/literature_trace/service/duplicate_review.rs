//! Fixed duplicate-review calls share the existing owned host and shutdown gate.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_duplicates::{DuplicateResources, ReviewRequest, ReviewResult},
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    data::DataRows,
};

mod decision;
mod errors;

impl LiteratureTraceService {
    pub(crate) async fn review_duplicates(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: ReviewRequest,
    ) -> NativeResult<ReviewResult> {
        let writes = matches!(&request, ReviewRequest::Decide(_));
        let owner = Arc::clone(self);
        let selected = key.clone();
        // This task, not the waiting caller, owns the original SQL and finisher.
        let result = tokio::spawn(async move {
            owner
                .duplicate_review_owned(database, &selected, request)
                .await
        })
        .await
        .map_err(|_| errors::unknown("重复项调用未能结算，禁止自动重试。", writes))
        .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if writes && error.duplicate_sql.is_none() {
                error.duplicate_sql = Some(
                    crate::runtime_plugins::literature_duplicates::DecisionSqlState::NotDispatched,
                );
            }
            error
        })
    }

    async fn duplicate_review_resources(&self, key: &str) -> NativeResult<DuplicateResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_duplicate_review(&catalog, &selected)?;
            DuplicateResources::load(&catalog, &package)
                .map_err(|cause| Self::error(cause.code, cause.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "重复项资源核对任务未完成。"))?
    }

    async fn duplicate_review_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: ReviewRequest,
    ) -> NativeResult<ReviewResult> {
        let resources = self.duplicate_review_resources(key).await?;
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
            self.available()?;
            let mut before = None;
            let result = async {
                if resources.package_key() != key {
                    return Err(Self::error(
                        "plugin_host_response_mismatch",
                        "重复项资源不属于本次插件包。",
                    ));
                }
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                match request {
                    ReviewRequest::Decide(request) => {
                        self.run_duplicate_decision(host, database, resources, request)
                            .await
                    }
                    read => {
                        self.run_duplicate_read(host, database, resources, read)
                            .await
                    }
                }
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|error| errors::scope_request(error, before.as_ref()))
        }
    }

    #[cfg(unix)]
    async fn run_duplicate_read(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        resources: DuplicateResources,
        request: ReviewRequest,
    ) -> NativeResult<ReviewResult> {
        let history = matches!(request, ReviewRequest::History);
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let rows = tokio::task::spawn_blocking(move || {
            let guard = database.lock().map_err(errors::owner)?;
            if history {
                resources.history(&guard)
            } else {
                resources.candidates(&guard)
            }
            .map_err(errors::data)
        })
        .await
        .map_err(|_| host.error("storage_error", "重复项读取任务未完成。"))?
        .map_err(|error| errors::bind(host, error))?;
        self.available()
            .map_err(|error| errors::bind(host, error))?;
        let input = if history {
            CatalogInput::ProjectMergeOperations { rows: rows.clone() }
        } else {
            CatalogInput::ProjectDuplicateCandidates { rows: rows.clone() }
        };
        let output = tokio::select! {
            biased;
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭重复项读取。")),
            output = host.catalog_call(input, Duration::from_secs(60)) => output,
        }?;
        match output {
            CatalogOutput::DuplicateCandidates { items } if !history => {
                if !corresponds(&rows, items.iter().map(|item| item.to_row())) {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "重复候选与实际读取的完整有序行不匹配。",
                    ));
                }
                Ok(ReviewResult::Candidates(
                    items.into_iter().map(|item| item.into_public()).collect(),
                ))
            }
            CatalogOutput::MergeOperations { items } if history => {
                if !corresponds(&rows, items.iter().map(|item| item.to_row())) {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "合并历史与实际读取的完整有序行不匹配。",
                    ));
                }
                Ok(ReviewResult::History(items))
            }
            _ => Err(host.error("plugin_host_response_mismatch", "重复项读取阶段不匹配。")),
        }
    }
}

fn corresponds(
    expected: &DataRows,
    actual: impl Iterator<Item = Vec<research_workbench_native_sdk::data::DataValue>>,
) -> bool {
    actual.eq(expected.rows.iter().cloned())
}

#[cfg(test)]
mod tests;
