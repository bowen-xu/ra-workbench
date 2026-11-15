//! Metadata uses the existing owned host and storage task; no parallel runtime.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        literature_catalog::{data::CatalogResources, save::SaveResources},
        literature_metadata::{
            MetadataFailure, MetadataPhaseOutcome, MetadataProblem, MetadataResources,
        },
    },
};
use research_workbench_native_sdk::metadata::{MetadataRequestV1, MetadataResultV1};

mod errors;
#[cfg(unix)]
mod run;

struct MetadataStorage {
    database: SharedDatabase,
    metadata: MetadataResources,
    catalog: CatalogResources,
    save: SaveResources,
}

impl LiteratureTraceService {
    pub(crate) async fn apply_metadata(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: MetadataRequestV1,
    ) -> NativeResult<MetadataResultV1> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let result =
            tokio::spawn(async move { owner.metadata_owned(database, &selected, request).await })
                .await
                .map_err(|_| errors::unknown("owner", "METADATA_OWNER_UNSETTLED"))
                .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.literature_metadata.is_none() {
                error.literature_metadata = Some(Box::new(MetadataFailure::new(
                    "preparation",
                    MetadataProblem::Owner("METADATA_NOT_DISPATCHED"),
                    MetadataPhaseOutcome::NotStarted,
                )));
            }
            error
        })
    }

    async fn metadata_storage(
        &self,
        database: SharedDatabase,
        key: &str,
    ) -> NativeResult<MetadataStorage> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_metadata(&catalog, &key)?;
            let failure = |error: crate::runtime_plugins::RuntimePluginError| {
                Self::error(error.code, error.message)
            };
            Ok(MetadataStorage {
                database,
                metadata: MetadataResources::load(&catalog, &package).map_err(failure)?,
                catalog: CatalogResources::load(&catalog, &package).map_err(failure)?,
                save: SaveResources::load(&catalog, &package).map_err(failure)?,
            })
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_accept_failed",
                "元数据固定资源核对任务未完成。",
            )
        })?
    }

    async fn metadata_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: MetadataRequestV1,
    ) -> NativeResult<MetadataResultV1> {
        let storage = self.metadata_storage(database, key).await?;
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
            let mut slot = self
                .host
                .try_lock()
                .map_err(|_| Self::error("plugin_host_busy", "文献原生实例已有在途请求。"))?;
            self.available()?;
            let mut before = None;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_metadata(host, storage, request).await
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
