use super::*;
use crate::runtime_plugins::literature_import_jobs::{self as jobs, SqlOutcome};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    catalog_save::{CatalogSaveInput, CatalogSaveOutcome, PreparedDoi},
    import_jobs::{
        BibtexPreparationV1, ImportJobsProjectionInputV1, ImportJobsProjectionOutcomeV1,
        PrepareBibtexInputV1,
    },
};

impl LiteratureTraceService {
    pub(super) async fn run_import_jobs(
        &self,
        host: &mut NativeHost,
        storage: ImportStorage,
        request: ImportJobsRequestV1,
    ) -> NativeResult<ImportJobsResultV1> {
        let preparation = if let ImportJobsRequestV1::Bibtex { drafts } = &request {
            let inputs = drafts
                .iter()
                .cloned()
                .map(|draft| {
                    let display = crate::doi::clean_doi(&draft.doi)
                        .unwrap_or_else(|| draft.doi.trim().to_owned());
                    CatalogSaveInput {
                        doi: PreparedDoi {
                            normalized: crate::doi::normalized_doi(&display),
                            display,
                        },
                        draft,
                    }
                })
                .collect();
            let result = self
                .import_jobs_call(
                    host,
                    CatalogInput::PrepareBibtex {
                        input: PrepareBibtexInputV1 { inputs },
                    },
                )
                .await?;
            let CatalogOutput::BibtexPreparation { outcome } = result else {
                return Err(host.error(
                    "plugin_host_response_mismatch",
                    "BibTeX准备回复种类不匹配。",
                ));
            };
            match outcome {
                BibtexPreparationV1::Rejected { message, .. } if drafts.is_empty() => {
                    return Err(host.error("invalid_input", message));
                }
                BibtexPreparationV1::Ready { plan }
                    if plan.entries.len() == drafts.len() && !drafts.is_empty() =>
                {
                    for (entry, original) in plan.entries.iter().zip(drafts) {
                        let raw: research_workbench_native_sdk::catalog_save::CatalogDraft =
                            serde_json::from_str(&entry.raw_input).map_err(|_| {
                                host.error(
                                    "plugin_host_response_mismatch",
                                    "BibTeX原始输入未保持。",
                                )
                            })?;
                        if &raw != original
                            || matches!(&entry.save, CatalogSaveOutcome::Planned { plan } if plan.item.id != original.id)
                        {
                            return Err(host.error(
                                "plugin_host_response_mismatch",
                                "BibTeX输入或目标身份未保持。",
                            ));
                        }
                    }
                    Some(plan)
                }
                _ => {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "BibTeX准备与本次输入不匹配。",
                    ));
                }
            }
        } else {
            None
        };
        self.available()?;
        // Never select away from accepted SQL. The original guard spans all
        // per-entry transactions, autocommit updates and final raw job capture.
        let (input, progress) = tokio::task::spawn_blocking(move || {
            let mut db = storage.database.lock().map_err(owner_error)?;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Some(plan) = preparation {
                    let (writes, reads) = storage.save.as_ref().ok_or_else(|| {
                        Self::error("plugin_data_resource_unavailable", "BibTeX保存资源不可用。")
                    })?;
                    return jobs::execute::bibtex(
                        &mut db,
                        &storage.resources,
                        reads,
                        writes,
                        &plan,
                    );
                }
                let mut progress = ImportJobsProgress::new("job-read");
                let input = match request {
                    ImportJobsRequestV1::Get { id } => {
                        progress.job_id = Some(id.clone());
                        let rows = jobs::data::job(&db, &storage.resources, &id, &progress)?;
                        ImportJobsProjectionInputV1::Job {
                            expected_id: id,
                            rows,
                        }
                    }
                    ImportJobsRequestV1::List {} => {
                        jobs::data::list(&db, &storage.resources, &progress)?
                    }
                    ImportJobsRequestV1::Cancel { id } => {
                        progress.phase = "cancel";
                        progress.job_id = Some(id.clone());
                        progress.outcome = SqlOutcome::StatementFailed;
                        let affected_rows = storage
                            .resources
                            .write_on_connection(
                                &db,
                                "cancel",
                                vec![research_workbench_native_sdk::data::DataValue::Text(id)],
                            )
                            .map_err(|cause| progress.data(cause))?;
                        progress.outcome = SqlOutcome::Returned;
                        ImportJobsProjectionInputV1::Cancel { affected_rows }
                    }
                    ImportJobsRequestV1::Bibtex { .. } => return Err(progress.mismatch()),
                };
                Ok((input, progress))
            }));
            match result {
                Ok(result) => result,
                Err(payload) => {
                    db.require_recovery();
                    std::panic::resume_unwind(payload);
                }
            }
        })
        .await
        .map_err(|_| {
            let progress = ImportJobsProgress::unknown("execution");
            progress.bind(host.error(
                "storage_error",
                "导入数据库任务未结算；结果未知，禁止自动重试。",
            ))
        })?
        .map_err(|error| bind(host, error))?;
        let result = self
            .import_jobs_call(
                host,
                CatalogInput::ProjectImportJobs {
                    input: input.clone(),
                },
            )
            .await
            .map_err(|error| progress.bind(error))?;
        match result {
            CatalogOutput::ImportJobsProjection {
                outcome: ImportJobsProjectionOutcomeV1::Success { result },
            } => {
                let valid = match (&input, &result) {
                    (
                        ImportJobsProjectionInputV1::Job { expected_id, .. },
                        ImportJobsResultV1::Job { job },
                    ) => expected_id == &job.id,
                    (
                        ImportJobsProjectionInputV1::Jobs { expected_ids, .. },
                        ImportJobsResultV1::Jobs { jobs },
                    ) => expected_ids.iter().eq(jobs.iter().map(|job| &job.id)),
                    (
                        ImportJobsProjectionInputV1::Cancel { affected_rows },
                        ImportJobsResultV1::Done {},
                    ) => *affected_rows > 0,
                    _ => false,
                };
                if valid {
                    Ok(result)
                } else {
                    Err(bind(host, progress.mismatch()))
                }
            }
            CatalogOutput::ImportJobsProjection {
                outcome: ImportJobsProjectionOutcomeV1::NotFound { message },
            } => {
                let missing = match &input {
                    ImportJobsProjectionInputV1::Job { rows, .. } => rows.header.rows.is_empty(),
                    ImportJobsProjectionInputV1::Jobs { rows, .. } => {
                        rows.iter().any(|row| row.header.rows.is_empty())
                    }
                    ImportJobsProjectionInputV1::Cancel { affected_rows } => *affected_rows == 0,
                };
                if missing {
                    Err(progress.bind(host.error("not_found", message)))
                } else {
                    Err(bind(host, progress.mismatch()))
                }
            }
            _ => Err(bind(host, progress.mismatch())),
        }
    }

    async fn import_jobs_call(
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
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭导入操作。")),
            result = host.catalog_call(input, Duration::from_secs(60)) => result,
        }
    }
}

fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}
