use super::*;
use crate::runtime_plugins::literature_pdf_import::execute::{PendingPdfResult, SaveStage};
use research_workbench_native_sdk::{
    catalog::{CatalogProjectionInput, CatalogProjectionOutcome, CatalogRequest},
    catalog_save::{CatalogSaveInput, CatalogSaveOutcome, PreparedDoi},
};

impl LiteratureTraceService {
    pub(super) async fn pdf_worker(
        &self,
        storage: PdfStorage,
        key: &str,
        instance: &str,
        job: &str,
    ) -> NativeResult<()> {
        let running = storage.clone();
        let id = job.to_owned();
        pdf_block("pdf-running", move || execute::mark_running(&running, &id)).await?;
        let capture = storage.clone();
        let id = job.to_owned();
        // Preserve the original one complete pending snapshot, including raw input.
        let pending = pdf_block("pdf-pending", move || execute::pending(&capture, &id)).await?;
        for (index, result) in pending.into_iter().enumerate() {
            let mut slot = self.host.lock().await;
            let before = slot.as_ref().and_then(NativeHost::request_identity);
            let outcome = async {
                self.available()?;
                let host = pinned(&mut slot, key, instance)?;
                let current = storage.clone();
                let id = job.to_owned();
                if pdf_block("pdf-cancel-check", move || {
                    execute::observe_cancel(&current, &id)
                })
                .await
                .map_err(|error| bind(host, error))?
                {
                    return Ok(false);
                }
                self.pdf_item(host, storage.clone(), job, result, index)
                    .await?;
                Ok(true)
            }
            .await;
            let continuing = self
                .finish_domain(&mut slot, outcome)
                .await
                .map_err(|error| clear_previous(error, before.as_ref()))?;
            // Release this finite stage before status/cancel or the next item.
            drop(slot);
            if !continuing {
                return Ok(());
            }
        }
        let id = job.to_owned();
        pdf_block("pdf-finish", move || execute::finish(&storage, &id)).await
    }

    async fn pdf_item(
        &self,
        host: &mut NativeHost,
        storage: PdfStorage,
        job: &str,
        result: PendingPdfResult,
        index: usize,
    ) -> NativeResult<()> {
        let mut progress = execute::progress(job, "pdf-decode");
        progress.result_id = Some(result.id.clone());
        let output = self
            .pdf_call(
                host,
                CatalogInput::DecodePdfImportCandidate {
                    raw_input: result.raw_input.clone(),
                },
            )
            .await
            .map_err(|error| progress.bind(error))?;
        let CatalogOutput::PdfImportCandidateDecoded { outcome } = output else {
            return Err(progress.bind(mismatch(host)));
        };
        if outcome.raw_input != result.raw_input || outcome.candidate.draft.id.is_some() {
            return Err(progress.bind(mismatch(host)));
        }
        let candidate = outcome.candidate;
        let display = crate::doi::clean_doi(&candidate.draft.doi)
            .unwrap_or_else(|| candidate.draft.doi.trim().to_owned());
        let draft = candidate.draft;
        progress.phase = "pdf-save-prepare";
        let output = self
            .pdf_call(
                host,
                CatalogInput::PrepareSave {
                    input: Box::new(CatalogSaveInput {
                        doi: PreparedDoi {
                            normalized: crate::doi::normalized_doi(&display),
                            display,
                        },
                        draft,
                    }),
                },
            )
            .await
            .map_err(|error| progress.bind(error))?;
        let CatalogOutput::SavePlan { outcome } = output else {
            return Err(progress.bind(mismatch(host)));
        };
        if matches!(&outcome, CatalogSaveOutcome::Planned { plan } if plan.item.id.is_some()) {
            return Err(progress.bind(mismatch(host)));
        }
        progress.phase = "pdf-item-save";
        let save_storage = storage.clone();
        let (stage, saved_progress) = pdf_block("pdf-item-save", move || {
            let stage = execute::save(&save_storage, outcome, &mut progress)?;
            Ok((stage, progress))
        })
        .await
        .map_err(|error| bind(host, error))?;
        let mut progress = saved_progress;
        let item = match stage {
            SaveStage::Failed(message) => Err(message),
            SaveStage::Saved(rows) => {
                let id = rows.item_id;
                progress.phase = "pdf-save-readback";
                // Original save performed complete item_from after COMMIT. Its
                // failure never enters attachment work or parent cleanup.
                let missing = rows
                    .items
                    .first()
                    .is_none_or(|row| row.main.rows.is_empty());
                let output = self
                    .pdf_call(
                        host,
                        CatalogInput::Project {
                            input: CatalogProjectionInput {
                                request: CatalogRequest::Get { id: id.clone() },
                                items: rows.items,
                            },
                        },
                    )
                    .await
                    .map_err(|error| progress.bind(error))?;
                match output {
                    CatalogOutput::Projection {
                        outcome: CatalogProjectionOutcome::Success { items },
                    } if items.len() == 1 && items[0].id == id => {
                        progress.phase = "pdf-attach";
                        let attach_storage = storage.clone();
                        let path = candidate.path;
                        let (outcome, next) = pdf_block("pdf-attach", move || {
                            let outcome =
                                execute::attach(&attach_storage, &id, &path, &mut progress)?;
                            Ok((outcome, progress))
                        })
                        .await
                        .map_err(|error| bind(host, error))?;
                        progress = next;
                        outcome
                    }
                    CatalogOutput::Projection {
                        outcome: CatalogProjectionOutcome::NotFound,
                    } if missing => Err("找不到文献或附件".into()),
                    _ => return Err(progress.bind(mismatch(host))),
                }
            }
        };
        let record_storage = storage;
        pdf_block("pdf-result-update", move || {
            execute::record(&record_storage, &result.id, item, index, &mut progress)
        })
        .await
        .map_err(|error| bind(host, error))
    }
}

fn pinned<'a>(
    slot: &'a mut Option<NativeHost>,
    key: &str,
    instance: &str,
) -> NativeResult<&'a mut NativeHost> {
    let host = slot.as_mut().ok_or_else(|| {
        LiteratureTraceService::error(
            "plugin_host_instance_changed",
            "PDF导入原生实例已变化；此任务不会自动重放。",
        )
    })?;
    if host.package_key() != key
        || host.is_failed()
        || host
            .request_identity()
            .is_none_or(|(actual, _)| actual != instance)
    {
        let mut error = LiteratureTraceService::error(
            "plugin_host_instance_changed",
            "PDF导入原生实例已变化；此任务不会自动重放。",
        );
        error.instance_id = Some(instance.into());
        return Err(error);
    }
    Ok(host)
}
