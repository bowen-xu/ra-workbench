//! A catalog call reuses the same finite host, package and shutdown owner.
use super::*;
use crate::data_interface::{SharedDatabase, plugin_resources::PluginDataError};
use crate::runtime_plugins::literature_catalog::data::{
    CatalogResources, capture_items, capture_selection, capture_trashed_items, recheck_selection,
};
use research_workbench_native_sdk::catalog::{
    CatalogInput, CatalogItem, CatalogOutput, CatalogProjectionInput, CatalogProjectionOutcome,
    CatalogQueryPlan, CatalogRequest,
};

impl LiteratureTraceService {
    pub(crate) async fn catalog(
        &self,
        database: SharedDatabase,
        key: String,
        request: CatalogRequest,
    ) -> NativeResult<Vec<CatalogItem>> {
        self.catalog_with_visibility(database, key, request, CatalogVisibility::Active)
            .await
    }

    pub(crate) async fn list_trashed_catalog(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        query: Option<String>,
    ) -> NativeResult<Vec<CatalogItem>> {
        // Preserve the old entry: a purge error is logged, then a separately
        // acquired connection reads the remaining trash. No automatic replay.
        match self
            .purge_expired_trash(Arc::clone(&database), key.clone())
            .await
        {
            Ok(warnings) => {
                for warning in warnings {
                    eprintln!("回收站到期清理警告：{warning}");
                }
            }
            Err(error) => eprintln!("回收站到期清理失败：{error}"),
        }
        self.catalog_with_visibility(
            database,
            key,
            CatalogRequest::List { query },
            CatalogVisibility::Trash,
        )
        .await
    }

    async fn catalog_with_visibility(
        &self,
        database: SharedDatabase,
        key: String,
        request: CatalogRequest,
        visibility: CatalogVisibility,
    ) -> NativeResult<Vec<CatalogItem>> {
        self.available().map_err(|error| error.package(&key))?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_catalog(&catalog, &selected)?;
            match visibility {
                CatalogVisibility::Active => CatalogResources::load(&catalog, &package),
                CatalogVisibility::Trash => CatalogResources::load_trash(&catalog, &package),
            }
            .map_err(|error| Self::error(error.code, &error.message).package(&selected))
        })
        .await
        .map_err(|_| {
            Self::error("plugin_host_accept_failed", "文献目录包核对任务未完成。").package(&key)
        })??;
        #[cfg(not(unix))]
        {
            let _ = (database, request, resources);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献目录宿主。",
            )
            .package(&key))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            let shutdown = self.stopped.notified();
            tokio::pin!(shutdown);
            shutdown.as_mut().enable();
            self.available().map_err(|error| error.package(&key))?;
            let result = tokio::select! {
                biased;
                _ = &mut shutdown => Err(Self::error("plugin_host_stopping", "应用正在关闭文献插件。")),
                result = self.run_catalog(&mut slot, &key, database, request, resources, visibility) => result,
            };
            // A valid frame carrying the wrong projection is not a domain
            // rejection. Retire that connection before another explicit call.
            let invalid_response = result
                .as_ref()
                .is_err_and(|error| error.code == "plugin_host_response_mismatch");
            if result.is_err()
                && slot.as_ref().is_some_and(|host| {
                    host.is_failed() || self.stopping.load(Ordering::Acquire) || invalid_response
                })
                && let Some(mut host) = slot.take()
            {
                let cleanup = CleanupGuard::new(&self.cleanup_failed);
                let mut error = match result {
                    Err(error) => error,
                    Ok(_) => unreachable!("result checked above"),
                };
                host.finish_failed(&mut error).await;
                cleanup.finish(Some(error.as_ref()));
                return Err(error.package(&key));
            }
            result.map_err(|error| error.package(&key))
        }
    }

    #[cfg(unix)]
    async fn run_catalog(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        database: SharedDatabase,
        request: CatalogRequest,
        resources: CatalogResources,
        visibility: CatalogVisibility,
    ) -> NativeResult<Vec<CatalogItem>> {
        if resources.package_key() != key {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "目录资源不属于本次插件包。",
            ));
        }
        let host = self.ensure_host(slot, key).await?;
        let (expected_ids, items) = match &request {
            CatalogRequest::Get { id } => {
                let ids = vec![id.clone()];
                tokio::task::spawn_blocking(move || {
                    let db = database
                        .lock()
                        .map_err(|cause| Self::error("storage_error", cause.to_string()))?;
                    let items = item_rows(&db, &resources, &ids, visibility).map_err(data_error)?;
                    Ok((ids, items))
                })
                .await
                .map_err(|_| host.error("storage_error", "文献题录读取任务未完成。"))?
                .map_err(|error| bind_error(host, error))?
            }
            CatalogRequest::List { .. } => {
                let CatalogOutput::Plan { plan } = host
                    .catalog_call(
                        CatalogInput::Plan {
                            request: request.clone(),
                        },
                        Duration::from_secs(60),
                    )
                    .await?
                else {
                    return Err(
                        host.error("plugin_host_response_mismatch", "文献目录查询阶段不匹配。")
                    );
                };
                if matches!(plan, CatalogQueryPlan::Get { .. }) {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "列表请求不能替换成单项请求。",
                    ));
                }
                let capture_database = Arc::clone(&database);
                let capture_resources = resources.clone();
                let selection = tokio::task::spawn_blocking(move || {
                    let db = capture_database
                        .lock()
                        .map_err(|cause| Self::error("storage_error", cause.to_string()))?;
                    capture_selection(&db, &capture_resources, &plan).map_err(data_error)
                })
                .await
                .map_err(|_| host.error("storage_error", "文献目录选择读取任务未完成。"))?
                .map_err(|error| bind_error(host, error))?;
                let CatalogOutput::Selection { ids } = host
                    .catalog_call(
                        CatalogInput::Select {
                            selection: selection.rows().clone(),
                        },
                        Duration::from_secs(60),
                    )
                    .await?
                else {
                    return Err(
                        host.error("plugin_host_response_mismatch", "文献目录选择阶段不匹配。")
                    );
                };
                tokio::task::spawn_blocking(move || {
                    let db = database
                        .lock()
                        .map_err(|cause| Self::error("storage_error", cause.to_string()))?;
                    recheck_selection(&db, &selection).map_err(data_error)?;
                    let items = item_rows(&db, &resources, &ids, visibility).map_err(data_error)?;
                    Ok((ids, items))
                })
                .await
                .map_err(|_| host.error("storage_error", "文献题录读取任务未完成。"))?
                .map_err(|error| bind_error(host, error))?
            }
        };
        let CatalogOutput::Projection { outcome } = host
            .catalog_call(
                CatalogInput::Project {
                    input: CatalogProjectionInput {
                        request: request.clone(),
                        items,
                    },
                },
                Duration::from_secs(60),
            )
            .await?
        else {
            return Err(host.error("plugin_host_response_mismatch", "文献目录组装阶段不匹配。"));
        };
        if !projection_ids_match(&outcome, &expected_ids) {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "题录结果与本次捕获目标的身份、数量或顺序不匹配。",
            ));
        }
        match outcome {
            CatalogProjectionOutcome::Success { items } => Ok(items),
            CatalogProjectionOutcome::NotFound => Err(host.error("not_found", "找不到文献或附件")),
        }
    }
}

#[derive(Clone, Copy)]
enum CatalogVisibility {
    Active,
    Trash,
}

fn item_rows(
    db: &rusqlite::Connection,
    resources: &CatalogResources,
    ids: &[String],
    visibility: CatalogVisibility,
) -> Result<Vec<research_workbench_native_sdk::catalog::CatalogItemRows>, Box<PluginDataError>> {
    match visibility {
        CatalogVisibility::Active => capture_items(db, resources, ids),
        CatalogVisibility::Trash => capture_trashed_items(db, resources, ids),
    }
}

/// Bind DTO identity to the exact capture inputs, without redoing domain selection.
fn projection_ids_match(outcome: &CatalogProjectionOutcome, expected_ids: &[String]) -> bool {
    match outcome {
        CatalogProjectionOutcome::Success { items } => items
            .iter()
            .map(|item| item.id.as_str())
            .eq(expected_ids.iter().map(String::as_str)),
        CatalogProjectionOutcome::NotFound => true,
    }
}

fn data_error(cause: Box<PluginDataError>) -> Box<NativeHostError> {
    let code = if cause.code == "plugin_data_snapshot_changed" {
        cause.code
    } else {
        "storage_error"
    };
    let mut error =
        LiteratureTraceService::error(code, format!("{}：{}", cause.message, cause.safe_cause));
    error.data_error = Some(cause);
    error
}

#[cfg(unix)]
fn bind_error(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}

#[cfg(test)]
mod tests;
