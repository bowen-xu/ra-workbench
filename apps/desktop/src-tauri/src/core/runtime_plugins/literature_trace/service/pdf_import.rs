//! One existing Core-owned PDF worker; host access is bounded to finite stages.
use super::*;
use crate::{
    runtime_plugins::{
        literature_attachments::{AttachmentProgress, AttachmentResources},
        literature_catalog::{data::CatalogResources, save::SaveResources},
        literature_import_jobs::{ImportJobsProgress, ImportJobsResources},
        literature_pdf_import::{
            PdfImportResources,
            execute::{self, PdfStorage},
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    import_jobs::{ImportJobsProjectionOutcomeV1, ImportJobsResultV1, ImportedJob},
    pdf_import::{PdfImportCandidate, PdfImportHeaderPreparationV1},
};

#[cfg(unix)]
mod worker;

impl LiteratureTraceService {
    pub(crate) async fn start_pdf_import(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        candidates: Vec<PdfImportCandidate>,
    ) -> NativeResult<ImportedJob> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            // The same gate already used by DOI drains this worker on shutdown.
            // A stopped client drops only receive, never this admitted owner.
            let _operation = owner.doi_operations.read().await;
            let started = owner.pdf_header(store, &selected, candidates).await;
            let (job, storage, instance) = match started {
                Ok(value) => value,
                Err(error) => {
                    drop(send.send(Err(error)));
                    return Ok(());
                }
            };
            let job_id = job.id.clone();
            // Sending to a cancelled caller does not cancel the accepted job.
            drop(send.send(Ok(job)));
            #[cfg(unix)]
            if let Err(mut error) = owner
                .pdf_worker(storage.clone(), &selected, &instance, &job_id)
                .await
            {
                // The header succeeded before this worker was started, even
                // when a later SQL task loses its local progress on panic.
                if let Some(progress) = &mut error.import_jobs {
                    progress.job_id.get_or_insert_with(|| job_id.clone());
                    progress.header_committed = Some(true);
                }
                error.package_key = Some(selected.clone());
                // Unknown effects are not converted into an ordinary failed job.
                // A fenced storage owner also rejects fail() before either write.
                if unsettled(&error) {
                    return Err(error);
                }
                let original = error.clone();
                let failed = pdf_block("pdf-fail", move || {
                    execute::fail(&storage, &job_id, &original)
                })
                .await;
                return match failed {
                    Ok(()) => Err(error),
                    Err(failure) => Err(failure),
                };
            }
            #[cfg(not(unix))]
            let _ = (storage, instance, job_id);
            Ok::<_, Box<NativeHostError>>(())
        });
        receive
            .await
            .map_err(|_| unknown("pdf-owner").package(&key))?
            .map_err(|error| error.package(&key))
    }

    async fn pdf_storage(&self, store: AppStore, key: &str) -> NativeResult<PdfStorage> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        pdf_block("pdf-resources", move || {
            let package = package::accept_pdf_import(&catalog, &selected)?;
            let error = |cause: crate::runtime_plugins::RuntimePluginError| {
                Self::error(cause.code, cause.message)
            };
            Ok(PdfStorage {
                store,
                pdf: PdfImportResources::load(&catalog, &package).map_err(error)?,
                jobs: ImportJobsResources::load(&catalog, &package).map_err(error)?,
                reads: CatalogResources::load(&catalog, &package).map_err(error)?,
                writes: SaveResources::load(&catalog, &package).map_err(error)?,
                attachments: AttachmentResources::load(&catalog, &package).map_err(error)?,
            })
        })
        .await
    }

    async fn pdf_header(
        &self,
        store: AppStore,
        key: &str,
        candidates: Vec<PdfImportCandidate>,
    ) -> NativeResult<(ImportedJob, PdfStorage, String)> {
        let storage = self.pdf_storage(store, key).await?;
        if storage.pdf.package_key() != key {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "PDF导入资源不属于本次包。",
            ));
        }
        // Empty input historically rejects before acquiring the DB owner.
        // For a nonempty request preserve owner failure before pure preparation.
        if !candidates.is_empty() {
            let probe = storage.store.clone();
            pdf_block("pdf-owner", move || {
                drop(
                    probe
                        .connection()
                        .map_err(|cause| AttachmentProgress::new("owner").owner(cause).native())?,
                );
                Ok(())
            })
            .await?;
        }
        #[cfg(not(unix))]
        {
            let _ = (storage, candidates);
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
                let output = self
                    .pdf_call(
                        host,
                        CatalogInput::PreparePdfImportHeader {
                            candidates: candidates.clone(),
                        },
                    )
                    .await?;
                let CatalogOutput::PdfImportHeaderPreparation { outcome } = output else {
                    return Err(mismatch(host));
                };
                let entries = match *outcome {
                    PdfImportHeaderPreparationV1::Ready { entries }
                        if !candidates.is_empty() && entries.len() == candidates.len() =>
                    {
                        for (entry, candidate) in entries.iter().zip(&candidates) {
                            if &entry.candidate != candidate
                                || serde_json::from_str::<PdfImportCandidate>(&entry.raw_input)
                                    .ok()
                                    .as_ref()
                                    != Some(candidate)
                            {
                                return Err(mismatch(host));
                            }
                        }
                        entries
                    }
                    PdfImportHeaderPreparationV1::Rejected {
                        candidates: original,
                        reason,
                    } if candidates.is_empty() && original == candidates => {
                        return Err(host.error(reason.code(), reason.message()));
                    }
                    _ => return Err(mismatch(host)),
                };
                let initial = storage.clone();
                let (id, rows, progress) =
                    pdf_block("pdf-header", move || execute::create(&initial, &entries))
                        .await
                        .map_err(|error| bind(host, error))?;
                let output = self
                    .pdf_call(
                        host,
                        CatalogInput::ProjectImportJobs {
                            input: execute::header_input(id.clone(), rows),
                        },
                    )
                    .await
                    .map_err(|error| progress.bind(error))?;
                let job = match output {
                    CatalogOutput::ImportJobsProjection {
                        outcome:
                            ImportJobsProjectionOutcomeV1::Success {
                                result: ImportJobsResultV1::Job { job },
                            },
                    } if job.id == id => job,
                    _ => return Err(progress.bind(mismatch(host))),
                };
                let instance = host
                    .request_identity()
                    .ok_or_else(|| progress.bind(mismatch(host)))?
                    .0;
                Ok((job, storage, instance))
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|error| clear_previous(error, before.as_ref()))
        }
    }

    #[cfg(unix)]
    async fn pdf_call(
        &self,
        host: &mut NativeHost,
        input: CatalogInput,
    ) -> NativeResult<CatalogOutput> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available().map_err(|error| bind(host, error))?;
        tokio::select! {
            biased;
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭PDF导入准备。")),
            result = host.catalog_call(input, Duration::from_secs(60)) => result,
        }
    }
}

async fn pdf_block<T: Send + 'static>(
    phase: &'static str,
    work: impl FnOnce() -> NativeResult<T> + Send + 'static,
) -> NativeResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| unknown(phase))?
}
fn unknown(phase: &'static str) -> Box<NativeHostError> {
    ImportJobsProgress::unknown(phase).bind(LiteratureTraceService::error(
        "storage_error",
        "PDF导入工作未能结算；结果未知，禁止自动重试。",
    ))
}

#[cfg(unix)]
fn unsettled(error: &NativeHostError) -> bool {
    matches!(
        error.save_outcome,
        Some(crate::runtime_plugins::literature_catalog::save::SaveOutcome::Unknown)
    ) || error.import_jobs.as_ref().is_some_and(|progress| {
        matches!(
            progress.outcome,
            crate::runtime_plugins::literature_import_jobs::SqlOutcome::Unknown
        )
    }) || error.attachments.as_ref().is_some_and(|failure| {
        matches!(
            failure.progress.sql,
            crate::runtime_plugins::literature_attachments::AttachmentSql::Unknown
        )
    })
}
#[cfg(unix)]
fn mismatch(host: &NativeHost) -> Box<NativeHostError> {
    host.error(
        "plugin_host_response_mismatch",
        "PDF导入回复与本次原输入或阶段不匹配。",
    )
}
#[cfg(unix)]
fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}
#[cfg(unix)]
fn clear_previous(
    mut error: Box<NativeHostError>,
    before: Option<&(String, String)>,
) -> Box<NativeHostError> {
    if let Some((instance, request)) = before
        && error.instance_id.as_ref() == Some(instance)
        && error.request_id.as_ref() == Some(request)
    {
        error.request_id = None;
    }
    error
}
