//! Short current-item capture and final recheck; no DB guard spans a Native wait.
use super::{DoiStorage, owner_error, pinned_host, progress};
use crate::{
    data_interface::DatabaseGuard,
    runtime_plugins::{
        literature_catalog::data::capture_items,
        literature_doi::storage,
        literature_import_jobs::{ImportJobsProgress, SqlOutcome},
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeResult, transport::NativeHost},
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogItemRows, CatalogOutput},
    data::DataValue,
    doi_metadata::{DoiPreviewInputV1, DoiPreviewOutcomeV1, DoiTranslationV1, MetadataPreviewV1},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;

pub(super) struct CompleteDoi {
    pub(super) storage: DoiStorage,
    pub(super) key: String,
    pub(super) instance: String,
    pub(super) normalized: String,
    pub(super) item_id: Option<String>,
    pub(super) translation: DoiTranslationV1,
    pub(super) job_id: String,
    pub(super) result_id: String,
}

impl LiteratureTraceService {
    pub(super) async fn complete_doi(
        self: &Arc<Self>,
        input: CompleteDoi,
    ) -> NativeResult<MetadataPreviewV1> {
        let mut slot = self.host.lock().await;
        let before = slot.as_ref().and_then(NativeHost::request_identity);
        let phase = progress("candidate-read", &input.job_id, &input.result_id);
        let result = async {
            self.available().map_err(|error| phase.bind(error))?;
            let host = pinned_host(&mut slot, &input.key, &input.instance)
                .map_err(|error| phase.bind(error))?;
            if input.storage.resources.package_key() != input.key
                || input.storage.jobs.package_key() != input.key
            {
                return Err(bind(host, phase.mismatch()));
            }
            self.complete_current(host, input).await
        }
        .await;
        self.finish_domain(&mut slot, result)
            .await
            .map_err(|error| clear_previous(error, before.as_ref()))
    }

    async fn complete_current(
        &self,
        host: &mut NativeHost,
        input: CompleteDoi,
    ) -> NativeResult<MetadataPreviewV1> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut phase = progress("candidate-read", &input.job_id, &input.result_id);
        let mut current = capture(&input).await.map_err(|error| bind(host, error))?;
        let id = uuid::Uuid::new_v4().to_string();
        phase.phase = "candidate-projection";
        loop {
            let missing = current
                .as_ref()
                .is_some_and(|item| item.main.rows.is_empty());
            let current_normalized_doi =
                match current.as_ref().and_then(|item| item.main.rows.first()) {
                    Some(row) => match row.get(12) {
                        Some(DataValue::Text(doi)) => crate::doi::normalized_doi(doi),
                        _ => return Err(bind(host, phase.mismatch())),
                    },
                    None => String::new(),
                };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(phase.bind(host.error(
                    "plugin_host_timeout",
                    "DOI候选准备超过原60秒期限；此操作不会自动重放。",
                )));
            }
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available()
                .map_err(|error| phase.bind(bind(host, error)))?;
            let prepared = tokio::select! {
                biased;
                _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭DOI候选准备。")),
                output = host.catalog_call(CatalogInput::DoiPreview {
                    input: Box::new(DoiPreviewInputV1::Candidate {
                        id: id.clone(),
                        import_job_id: input.job_id.clone(),
                        item_id: input.item_id.clone(),
                        normalized_doi: input.normalized.clone(),
                        translation: Box::new(input.translation.clone()),
                        current: current.clone(),
                        current_normalized_doi,
                    }),
                }, remaining) => output,
            }.map_err(|error| phase.bind(error))?;
            let outcome = match prepared {
                CatalogOutput::DoiPreview { outcome } => *outcome,
                _ => return Err(bind(host, phase.mismatch())),
            };
            let (preview, raw_payload) = match outcome {
                DoiPreviewOutcomeV1::Rejected { code, message }
                    if missing && code == "not_found" =>
                {
                    // The original missing-item read is already linearized at
                    // capture; no unrelated current-state change repairs it.
                    return Err(phase.bind(host.error("not_found", message)));
                }
                DoiPreviewOutcomeV1::Candidate {
                    preview,
                    raw_payload,
                } if !missing => {
                    let mut expected = input.translation.draft.clone();
                    expected.id.clone_from(&input.item_id);
                    if preview.id != id
                        || preview.import_job_id != input.job_id
                        || preview.item_id != input.item_id
                        || preview.source_reference != input.normalized
                        || preview.source_kind != input.translation.source_kind
                        || preview.draft != expected
                        || preview.warnings != input.translation.warnings
                        || raw_payload != input.translation.raw_payload
                    {
                        return Err(bind(host, phase.mismatch()));
                    }
                    (preview, raw_payload)
                }
                _ => return Err(bind(host, phase.mismatch())),
            };
            self.available()
                .map_err(|error| phase.bind(bind(host, error)))?;
            // Once accepted, this writer is not selected away on stop/cancel.
            // Recheck and both original autocommit statements share one guard.
            let storage = input.storage.clone();
            let item_id = input.item_id.clone();
            let result_id = input.result_id.clone();
            let job_id = input.job_id.clone();
            let observed = current;
            let settled = tokio::task::spawn_blocking(move || {
                let progress = progress("candidate-recheck", &job_id, &result_id);
                let mut db = storage
                    .database
                    .lock()
                    .map_err(|cause| progress.bind(owner_error(cause)))?;
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let actual = read_current(&db, &storage, item_id.as_ref(), &progress)?;
                    if actual != observed {
                        return Ok(Completion::Changed(actual));
                    }
                    storage::insert_candidate(
                        &db,
                        &storage.resources,
                        &preview,
                        &raw_payload,
                        &result_id,
                    )?;
                    Ok(Completion::Inserted(Box::new(preview)))
                }));
                match result {
                    Ok(result) => result,
                    Err(_) => {
                        db.require_recovery();
                        Err(unknown(&job_id, &result_id))
                    }
                }
            })
            .await
            .map_err(|_| bind(host, unknown(&input.job_id, &input.result_id)))?
            .map_err(|error| bind(host, error))?;
            match settled {
                Completion::Inserted(preview) => return Ok(*preview),
                // Only pure preparation is repeated, under the same instance,
                // IDs and deadline. HTTP and dispatched SQL are never replayed.
                Completion::Changed(actual) => current = actual,
            }
        }
    }
}

enum Completion {
    Changed(Option<CatalogItemRows>),
    Inserted(Box<MetadataPreviewV1>),
}

async fn capture(input: &CompleteDoi) -> NativeResult<Option<CatalogItemRows>> {
    let storage = input.storage.clone();
    let item_id = input.item_id.clone();
    let phase = progress("candidate-read", &input.job_id, &input.result_id);
    tokio::task::spawn_blocking(move || {
        let db = storage
            .database
            .lock()
            .map_err(|cause| phase.bind(owner_error(cause)))?;
        read_current(&db, &storage, item_id.as_ref(), &phase)
    })
    .await
    .map_err(|_| {
        progress("candidate-read", &input.job_id, &input.result_id).bind(
            LiteratureTraceService::error("storage_error", "DOI当前文献读取任务未完成。"),
        )
    })?
}

fn read_current(
    db: &DatabaseGuard<'_>,
    storage: &DoiStorage,
    item_id: Option<&String>,
    phase: &ImportJobsProgress,
) -> NativeResult<Option<CatalogItemRows>> {
    match item_id {
        Some(id) => {
            let mut rows = capture_items(db, &storage.catalog, std::slice::from_ref(id))
                .map_err(|cause| phase.data(cause))?;
            if rows.len() != 1 {
                return Err(phase.mismatch());
            }
            Ok(rows.pop())
        }
        None => Ok(None),
    }
}

fn unknown(job_id: &str, result_id: &str) -> Box<NativeHostError> {
    let mut phase = progress("candidate-write", job_id, result_id);
    phase.outcome = SqlOutcome::Unknown;
    phase.candidate_insert_returned = None;
    phase.bind(LiteratureTraceService::error(
        "storage_error",
        "DOI候选写入任务未结算；结果未知，禁止自动重试。",
    ))
}

fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    error.package_key = Some(host.package_key().to_owned());
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}

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
