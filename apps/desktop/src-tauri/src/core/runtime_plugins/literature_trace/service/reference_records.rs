//! Original reference-record calls share the existing host and accepted task owner.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_reference_records::{
        self, ReferenceFailure, ReferenceProblem, ReferenceProgress, ReferenceRequest,
        ReferenceResources, ReferenceResult,
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    reference_records::{
        ReferenceRecordPreparationV1, ReferenceRecordPrepareInputV1, ReferenceRecordRejectionV1,
    },
};

struct Storage {
    database: SharedDatabase,
    resources: ReferenceResources,
}

impl LiteratureTraceService {
    pub(crate) async fn reference_records(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: ReferenceRequest,
    ) -> NativeResult<ReferenceResult> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        // Dropping the caller's wait does not abort an accepted SQL operation.
        tokio::spawn(async move {
            owner
                .reference_records_owned(database, &selected, request)
                .await
        })
        .await
        .map_err(|_| ReferenceFailure::unknown("execution", "REFERENCE_TASK_UNSETTLED").native())
        .and_then(|result| result)
        .map_err(|mut error| {
            error.package_key = Some(key);
            if error.reference_records.is_none() {
                let progress = ReferenceProgress::new("preparation");
                error.reference_records = Some(Box::new(
                    progress.failure(ReferenceProblem::Owner("REFERENCE_NOT_DISPATCHED")),
                ));
            }
            error
        })
    }

    async fn reference_records_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: ReferenceRequest,
    ) -> NativeResult<ReferenceResult> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let storage = tokio::task::spawn_blocking(move || {
            let package = package::accept_reference_records(&catalog, &selected)?;
            let resources = ReferenceResources::load(&catalog, &package)
                .map_err(|cause| Self::error(cause.code, cause.message))?;
            if resources.package_key() != selected {
                return Err(ReferenceProgress::new("resources")
                    .failure(ReferenceProblem::Binding)
                    .native());
            }
            // The original command obtained its Store guard before pure validation.
            drop(
                database
                    .lock()
                    .map_err(|cause| ReferenceFailure::owner(cause).native())?,
            );
            Ok::<_, Box<NativeHostError>>(Storage {
                database,
                resources,
            })
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "参考记录资源核对任务未完成。"))??;
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
            let mut slot = self.host.lock().await;
            self.available()?;
            let mut before = None;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_reference_records(host, storage, request)
                    .await
                    .map_err(|error| bind(host, error))
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
    async fn run_reference_records(
        &self,
        host: &mut NativeHost,
        storage: Storage,
        request: ReferenceRequest,
    ) -> NativeResult<ReferenceResult> {
        match request {
            ReferenceRequest::List { paper_id } => {
                self.available()?;
                // Only the original fixed resource order and structural codecs
                // are needed for reads; no speculative Native projection batch.
                tokio::task::spawn_blocking(move || {
                    let guard = storage
                        .database
                        .lock()
                        .map_err(|cause| ReferenceFailure::owner(cause).native())?;
                    literature_reference_records::list(&guard, &storage.resources, &paper_id)
                        .map(ReferenceResult::Records)
                        .map_err(ReferenceFailure::native)
                })
                .await
                .map_err(|cause| {
                    let category = if cause.is_panic() {
                        "REFERENCE_READ_PANICKED"
                    } else {
                        "REFERENCE_READ_CANCELLED"
                    };
                    let mut error = ReferenceProgress::new("execution")
                        .failure(ReferenceProblem::Owner(category))
                        .native();
                    error.code = "db_query_worker_failed".into();
                    error.message = format!("参考文献查询工作线程失败：{category}");
                    error
                })?
            }
            ReferenceRequest::Save { draft } => {
                let draft = *draft;
                let input = ReferenceRecordPrepareInputV1 {
                    draft: draft.clone(),
                };
                let stopped = self.stopped.notified();
                tokio::pin!(stopped);
                stopped.as_mut().enable();
                self.available()?;
                let output = tokio::select! {
                    biased;
                    _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭参考记录准备。")),
                    result = host.catalog_call(CatalogInput::PrepareReferenceRecord { input: Box::new(input) }, Duration::from_secs(60)) => result,
                }?;
                let CatalogOutput::ReferenceRecordPreparation { outcome } = output else {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "参考记录准备回复种类不匹配。",
                    ));
                };
                let plan = match *outcome {
                    ReferenceRecordPreparationV1::Ready { plan } if plan.draft == draft => plan,
                    ReferenceRecordPreparationV1::Rejected { reason }
                        if matches!(
                            reason,
                            ReferenceRecordRejectionV1::InvalidFields
                                | ReferenceRecordRejectionV1::InvalidStates
                                | ReferenceRecordRejectionV1::InvalidMetadataFields
                        ) =>
                    {
                        let error = tokio::task::spawn_blocking(move || {
                            let _guard =
                                storage.database.lock().map_err(ReferenceFailure::owner)?;
                            Ok::<_, ReferenceFailure>(
                                ReferenceProgress::new("validation")
                                    .failure(ReferenceProblem::Domain(reason)),
                            )
                        })
                        .await
                        .map_err(|_| {
                            ReferenceFailure::unknown(
                                "validation",
                                "REFERENCE_VALIDATION_UNSETTLED",
                            )
                            .native()
                        })?
                        .map_err(ReferenceFailure::native)?;
                        return Err(error.native());
                    }
                    _ => {
                        return Err(host.error(
                            "plugin_host_response_mismatch",
                            "参考记录准备与本次原输入不匹配。",
                        ));
                    }
                };
                self.available()?;
                // The same mutex is held through all original reads, Deferred
                // settlement and postcommit codec. No cancellation drops this writer.
                tokio::task::spawn_blocking(move || {
                    let mut guard = storage
                        .database
                        .lock()
                        .map_err(|cause| ReferenceFailure::owner(cause).native())?;
                    literature_reference_records::save(&mut guard, &storage.resources, &plan)
                        .map(|record| ReferenceResult::Record(Box::new(record)))
                        .map_err(ReferenceFailure::native)
                })
                .await
                .map_err(|_| {
                    ReferenceFailure::unknown("execution", "REFERENCE_WRITER_UNSETTLED").native()
                })?
            }
        }
    }
}

#[cfg(unix)]
fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}

#[cfg(test)]
mod tests;
