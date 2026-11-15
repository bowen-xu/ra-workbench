//! The existing host mutex owns both preparation and the original SQL batch.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        literature_catalog::{data::CatalogResources, save::SaveResources},
        literature_import_jobs::{ImportJobsProgress, ImportJobsResources},
    },
};
use research_workbench_native_sdk::import_jobs::{ImportJobsRequestV1, ImportJobsResultV1};

#[cfg(unix)]
mod run;
#[cfg(test)]
mod tests;

struct ImportStorage {
    database: SharedDatabase,
    resources: ImportJobsResources,
    save: Option<(SaveResources, CatalogResources)>,
}

impl LiteratureTraceService {
    pub(crate) async fn apply_import_jobs(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: ImportJobsRequestV1,
    ) -> NativeResult<ImportJobsResultV1> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        // Caller cancellation drops only this wait. Accepted work retains the
        // same host mutex until Core storage and the result capture settle.
        let result =
            tokio::spawn(
                async move { owner.import_jobs_owned(database, &selected, request).await },
            )
            .await
            .map_err(|_| {
                let progress = ImportJobsProgress::unknown("owner");
                progress.bind(Self::error(
                    "storage_error",
                    "导入操作结果未知，禁止自动重试。",
                ))
            })
            .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.import_jobs.is_none() {
                error.import_jobs = Some(Box::new(ImportJobsProgress::new("preparation")));
            }
            error
        })
    }

    async fn import_jobs_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: ImportJobsRequestV1,
    ) -> NativeResult<ImportJobsResultV1> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let bibtex = matches!(request, ImportJobsRequestV1::Bibtex { .. });
        let storage = tokio::task::spawn_blocking(move || {
            let package = if bibtex {
                package::accept_bibtex(&catalog, &selected)?
            } else {
                package::accept_import_jobs(&catalog, &selected)?
            };
            let error = |cause: crate::runtime_plugins::RuntimePluginError| {
                Self::error(cause.code, cause.message)
            };
            let resources = ImportJobsResources::load(&catalog, &package).map_err(error)?;
            let save = if bibtex {
                Some((
                    SaveResources::load(&catalog, &package).map_err(error)?,
                    CatalogResources::load(&catalog, &package).map_err(error)?,
                ))
            } else {
                None
            };
            // Preserve the original connection-admission failure before empty
            // input validation. This is not a dummy transaction or read snapshot.
            drop(database.lock().map_err(owner_error)?);
            Ok::<_, Box<NativeHostError>>(ImportStorage {
                database,
                resources,
                save,
            })
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "导入资源核对任务未完成。"))??;
        #[cfg(not(unix))]
        {
            let _ = (storage, request);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            // Status/cancel requests wait behind only the current PDF stage.
            // BibTeX retains its existing finite-batch busy admission boundary.
            let mut slot = if bibtex {
                self.host
                    .try_lock()
                    .map_err(|_| Self::error("plugin_host_busy", "文献原生实例已有在途请求。"))?
            } else {
                self.host.lock().await
            };
            self.available()?;
            let mut before = None;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_import_jobs(host, storage, request).await
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
}

pub(super) fn owner_error(cause: crate::data_interface::DataError) -> Box<NativeHostError> {
    use crate::data_interface::DataError;
    let category = match &cause {
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Sql(_) => "DATA_RUNTIME_STORAGE",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    let mut error =
        LiteratureTraceService::error("storage_error", format!("Core数据拥有者不可用：{category}"));
    if let DataError::Sql(sql) = cause {
        error.save_database_error = Some(Box::new(
            crate::runtime_plugins::native::SafeDatabaseError::sql("owner", &sql),
        ));
    }
    error
}
