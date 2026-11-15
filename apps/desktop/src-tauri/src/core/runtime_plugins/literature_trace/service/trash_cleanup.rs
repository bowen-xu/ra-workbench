//! One pure cleanup plan precedes the original full purge guard and item commits.
use super::*;
use crate::{
    data_interface::{DataError, SharedDatabase},
    runtime_plugins::literature_trash_cleanup::{
        self, TrashCleanupFailure, TrashCleanupOutcome, TrashCleanupProblem, TrashCleanupResources,
    },
};
use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput};

impl LiteratureTraceService {
    pub(crate) async fn purge_expired_trash(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
    ) -> NativeResult<Vec<String>> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        // Dropping a view's future does not cancel an accepted item transaction.
        // Shutdown waits on this same service's existing host/task owner.
        tokio::spawn(async move { owner.purge_owned(database, &selected).await })
            .await
            .map_err(|_| {
                TrashCleanupFailure::new(
                    "owner",
                    TrashCleanupProblem::Owner("TRASH_CLEANUP_WORKER_UNSETTLED"),
                    TrashCleanupOutcome::Unknown,
                )
                .native()
            })
            .and_then(|result| result)
            .map_err(|error| error.package(&key))
    }

    async fn trash_cleanup_resources(&self, key: &str) -> NativeResult<TrashCleanupResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_trash_cleanup(&catalog, &selected)?;
            TrashCleanupResources::load(&catalog, &package)
                .map_err(|cause| Self::error(cause.code, cause.message))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_accept_failed",
                "文献到期清理资源核对任务未完成。",
            )
        })?
    }

    async fn purge_owned(&self, database: SharedDatabase, key: &str) -> NativeResult<Vec<String>> {
        let resources = self.trash_cleanup_resources(key).await?;
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
            self.available()?;
            let mut before = None;
            let result = async {
                if resources.package_key() != key {
                    return Err(Self::error(
                        "plugin_host_response_mismatch",
                        "清理资源不属于本次插件包。",
                    ));
                }
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_trash_cleanup(host, database, resources).await
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|mut error| {
                    if let Some((instance, request)) = before.as_ref()
                        && error.instance_id.as_ref() == Some(instance)
                        && error.request_id.as_ref() == Some(request)
                    {
                        error.request_id = None;
                    }
                    error
                })
        }
    }

    #[cfg(unix)]
    async fn run_trash_cleanup(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        resources: TrashCleanupResources,
    ) -> NativeResult<Vec<String>> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let output = tokio::select! {
            biased;
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭文献到期清理准备。")),
            output = host.catalog_call(CatalogInput::PrepareTrashCleanup {}, Duration::from_secs(60)) => output,
        }?;
        let CatalogOutput::TrashCleanupPreparation { outcome } = output else {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "文献到期清理准备结果种类不匹配。",
            ));
        };
        let plan = literature_trash_cleanup::check_preparation(outcome)
            .map_err(|failure| bind(host, failure.native()))?;
        self.available().map_err(|error| bind(host, error))?;
        // There is no further host wait between captured expired IDs or item
        // commits: the original single DB guard owns the whole purge loop.
        tokio::task::spawn_blocking(move || {
            let mut guard = database.lock().map_err(owner_error)?;
            literature_trash_cleanup::purge(&mut guard, &resources, &plan)
                .map_err(TrashCleanupFailure::native)
        })
        .await
        .map_err(|_| {
            bind(
                host,
                TrashCleanupFailure::new(
                    "owner",
                    TrashCleanupProblem::Owner("TRASH_CLEANUP_WRITER_UNSETTLED"),
                    TrashCleanupOutcome::Unknown,
                )
                .native(),
            )
        })?
        .map_err(|error| bind(host, error))
    }
}

fn owner_error(cause: DataError) -> Box<NativeHostError> {
    let problem = match cause {
        DataError::Sql(error) => TrashCleanupProblem::Database(Box::new(
            crate::runtime_plugins::native::SafeDatabaseError::sql("owner", &error),
        )),
        DataError::Unavailable => TrashCleanupProblem::Owner("DATA_RUNTIME_UNAVAILABLE"),
        DataError::Pair(_) => TrashCleanupProblem::Owner("DATA_RUNTIME_PAIR_INVALID"),
        DataError::Log(_) => TrashCleanupProblem::Owner("DATA_RUNTIME_LOG"),
        DataError::Admission(_) => TrashCleanupProblem::Owner("DATA_RUNTIME_ADMISSION"),
    };
    TrashCleanupFailure::new("owner", problem, TrashCleanupOutcome::NotStarted).native()
}

#[cfg(unix)]
fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}
