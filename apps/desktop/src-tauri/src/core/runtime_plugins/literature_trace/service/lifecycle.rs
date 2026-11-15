//! The existing service owns preparation, accepted SQL and shutdown settlement.
use super::*;
use crate::{
    data_interface::{DataError, SharedDatabase},
    runtime_plugins::literature_lifecycle::{
        self, LifecycleFailure, LifecycleOutcome, LifecycleProblem, LifecycleResources,
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    literature_lifecycle::{LifecyclePreparationV1, LifecycleRequestV1},
};

impl LiteratureTraceService {
    pub(crate) async fn apply_literature_lifecycle(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: LifecycleRequestV1,
    ) -> NativeResult<()> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let batch = matches!(request, LifecycleRequestV1::TrashMany { .. });
        let result =
            tokio::spawn(async move { owner.lifecycle_owned(database, &selected, request).await })
                .await
                .map_err(|_| {
                    LifecycleFailure::new(
                        "owner",
                        LifecycleProblem::Owner("LIFECYCLE_WORKER_UNSETTLED"),
                        LifecycleOutcome::Unknown,
                    )
                    .native(batch)
                })
                .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.literature_lifecycle.is_none() {
                error.literature_lifecycle = Some(Box::new(LifecycleFailure::new(
                    "preparation",
                    LifecycleProblem::Owner("LIFECYCLE_NOT_DISPATCHED"),
                    LifecycleOutcome::NotDispatched,
                )));
            }
            error
        })
    }

    async fn lifecycle_resources(&self, key: &str) -> NativeResult<LifecycleResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_literature_lifecycle(&catalog, &selected)?;
            LifecycleResources::load(&catalog, &package)
                .map_err(|cause| Self::error(cause.code, cause.message))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_accept_failed",
                "文献生命周期资源核对任务未完成。",
            )
        })?
    }

    async fn lifecycle_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: LifecycleRequestV1,
    ) -> NativeResult<()> {
        let resources = self.lifecycle_resources(key).await?;
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
            let mut slot = self
                .host
                .try_lock()
                .map_err(|_| Self::error("plugin_host_busy", "文献原生实例已有在途请求。"))?;
            self.available()?;
            let mut before = None;
            let result = async {
                if resources.package_key() != key {
                    return Err(Self::error(
                        "plugin_host_response_mismatch",
                        "文献生命周期资源不属于本次插件包。",
                    ));
                }
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_lifecycle(host, database, resources, request).await
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
    async fn run_lifecycle(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        resources: LifecycleResources,
        request: LifecycleRequestV1,
    ) -> NativeResult<()> {
        let batch = matches!(request, LifecycleRequestV1::TrashMany { .. });
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        // No database guard exists while waiting for the pure preparation.
        let output = tokio::select! {
            biased;
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭文献生命周期准备。")),
            output = host.catalog_call(CatalogInput::PrepareLiteratureLifecycle { request: request.clone() }, Duration::from_secs(60)) => output,
        }?;
        let ids = match output {
            CatalogOutput::LiteratureLifecyclePreparation {
                outcome:
                    LifecyclePreparationV1::Ready {
                        request: prepared,
                        trash_item_ids,
                    },
            } => {
                literature_lifecycle::check_preparation(&request, &prepared, &trash_item_ids)
                    .map_err(|failure| bind(host, failure.native(batch)))?;
                trash_item_ids
            }
            CatalogOutput::LiteratureLifecyclePreparation {
                outcome: LifecyclePreparationV1::Rejected { error },
            } => {
                // The old batch entry acquired the owner even for empty input.
                // Keep an unavailable owner ahead of the domain rejection.
                tokio::task::spawn_blocking(move || {
                    database
                        .lock()
                        .map(drop)
                        .map_err(|cause| owner_error(cause, batch))
                })
                .await
                .map_err(|_| {
                    bind(
                        host,
                        LifecycleFailure::new(
                            "owner",
                            LifecycleProblem::Owner("LIFECYCLE_OWNER_CHECK_FAILED"),
                            LifecycleOutcome::NotDispatched,
                        )
                        .native(batch),
                    )
                })?
                .map_err(|error| bind(host, error))?;
                return Err(host.error(error.code.as_str(), error.message));
            }
            _ => {
                return Err(host.error(
                    "plugin_host_response_mismatch",
                    "文献生命周期准备结果种类不匹配。",
                ));
            }
        };
        self.available().map_err(|error| bind(host, error))?;
        // Accepted SQL belongs to this owned task; caller cancellation and stop
        // do not drop it. There is no postcommit host call or automatic replay.
        tokio::task::spawn_blocking(move || {
            let mut guard = database.lock().map_err(|cause| owner_error(cause, batch))?;
            literature_lifecycle::apply(&mut guard, &resources, &request, &ids)
                .map_err(|failure| failure.native(batch))
        })
        .await
        .map_err(|_| {
            bind(
                host,
                LifecycleFailure::new(
                    "execution",
                    LifecycleProblem::Owner("LIFECYCLE_WRITER_UNSETTLED"),
                    LifecycleOutcome::Unknown,
                )
                .native(batch),
            )
        })?
        .map_err(|error| bind(host, error))
    }
}

fn owner_error(cause: DataError, batch: bool) -> Box<NativeHostError> {
    let problem = match cause {
        DataError::Sql(error) => LifecycleProblem::Database(Box::new(
            crate::runtime_plugins::native::SafeDatabaseError::sql("owner", &error),
        )),
        DataError::Unavailable => LifecycleProblem::Owner("DATA_RUNTIME_UNAVAILABLE"),
        DataError::Pair(_) => LifecycleProblem::Owner("DATA_RUNTIME_PAIR_INVALID"),
        DataError::Log(_) => LifecycleProblem::Owner("DATA_RUNTIME_LOG"),
        DataError::Admission(_) => LifecycleProblem::Owner("DATA_RUNTIME_ADMISSION"),
    };
    LifecycleFailure::new("owner", problem, LifecycleOutcome::NotDispatched).native(batch)
}

#[cfg(unix)]
fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}
