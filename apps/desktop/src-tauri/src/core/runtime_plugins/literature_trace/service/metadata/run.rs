use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::CommittedCatalogRows,
    literature_metadata::{self, MetadataExecution, capture},
};
use research_workbench_native_sdk::{
    catalog::{
        CatalogInput, CatalogItem, CatalogOutput, CatalogProjectionInput, CatalogProjectionOutcome,
        CatalogRequest,
    },
    metadata::MetadataPreparationV1,
};

impl LiteratureTraceService {
    pub(super) async fn run_metadata(
        &self,
        host: &mut NativeHost,
        storage: MetadataStorage,
        request: MetadataRequestV1,
    ) -> NativeResult<MetadataResultV1> {
        let observed = {
            let database = Arc::clone(&storage.database);
            let metadata = storage.metadata.clone();
            let catalog = storage.catalog.clone();
            tokio::task::spawn_blocking(move || {
                let guard = database.lock().map_err(errors::owner)?;
                capture::capture(&guard, &metadata, &catalog, request)
                    .map_err(MetadataFailure::native)
            })
            .await
            .map_err(|_| errors::unknown("capture", "METADATA_CAPTURE_UNSETTLED"))?
            .map_err(|error| errors::bind(host, error))?
        };
        let current = match observed.projection() {
            Some(input) => Some(self.metadata_project(host, input).await?),
            None => None,
        };
        let input = observed
            .input(current)
            .map_err(|failure| errors::bind(host, failure.native()))?;
        let output = self
            .metadata_call(
                host,
                CatalogInput::PrepareMetadata {
                    input: Box::new(input.clone()),
                },
            )
            .await?;
        let prepared = match output {
            CatalogOutput::MetadataPreparation { outcome } => outcome,
            _ => {
                return Err(host.error(
                    "plugin_host_response_mismatch",
                    "元数据准备回复种类不匹配。",
                ));
            }
        };
        literature_metadata::binding::check(&input, &prepared)
            .map_err(|failure| errors::bind(host, failure.native()))?;
        self.available()
            .map_err(|error| errors::bind(host, error))?;
        let full = matches!(prepared, MetadataPreparationV1::FullApply { .. });
        // No select/cancellation drops this accepted SQL task. Shutdown shares
        // the host mutex until recheck, both original commits and capture settle.
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = storage.database.lock().map_err(errors::owner)?;
            capture::recheck(&guard, &storage.metadata, &storage.catalog, &observed)
                .map_err(MetadataFailure::native)?;
            literature_metadata::apply(
                &mut guard,
                &storage.metadata,
                &storage.catalog,
                &storage.save,
                &prepared,
            )
            .map_err(MetadataFailure::native)
        })
        .await
        .map_err(|_| {
            errors::bind(
                host,
                errors::unknown("execution", "METADATA_WRITER_UNSETTLED"),
            )
        })?
        .map_err(|error| errors::bind(host, error))?;
        match result {
            MetadataExecution::Done => Ok(MetadataResultV1::Done {}),
            MetadataExecution::Item(CommittedCatalogRows { item_id, items }) => {
                let input = CatalogProjectionInput {
                    request: CatalogRequest::Get {
                        id: item_id.clone(),
                    },
                    items,
                };
                self.metadata_project(host, input)
                    .await
                    .map(|item| MetadataResultV1::Item {
                        item: Box::new(item),
                    })
                    .map_err(|error| errors::committed(error, &item_id, full))
            }
        }
    }

    async fn metadata_project(
        &self,
        host: &mut NativeHost,
        input: CatalogProjectionInput,
    ) -> NativeResult<CatalogItem> {
        let id = match &input.request {
            CatalogRequest::Get { id } => id.clone(),
            _ => {
                return Err(host.error(
                    "plugin_host_response_mismatch",
                    "元数据结果只接受原单项投影。",
                ));
            }
        };
        match self
            .metadata_call(host, CatalogInput::Project { input })
            .await?
        {
            CatalogOutput::Projection {
                outcome: CatalogProjectionOutcome::Success { mut items },
            } if items.len() == 1 && items[0].id == id => Ok(items.remove(0)),
            CatalogOutput::Projection {
                outcome: CatalogProjectionOutcome::NotFound,
            } => {
                let mut failure = MetadataFailure::new(
                    "projection",
                    MetadataProblem::Domain(
                        research_workbench_native_sdk::metadata::MetadataRejectionV1::NotFound {},
                    ),
                    MetadataPhaseOutcome::NotStarted,
                );
                failure.item_id = Some(id);
                Err(errors::bind(host, failure.native()))
            }
            _ => Err(host.error(
                "plugin_host_response_mismatch",
                "元数据单项投影与本次读取不匹配。",
            )),
        }
    }

    async fn metadata_call(
        &self,
        host: &mut NativeHost,
        input: CatalogInput,
    ) -> NativeResult<CatalogOutput> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()
            .map_err(|error| errors::bind(host, error))?;
        tokio::select! {
            biased;
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭文献元数据准备。")),
            result = host.catalog_call(input, Duration::from_secs(60)) => result,
        }
    }
}
