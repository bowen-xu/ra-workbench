//! Original independent PDF job writes; no Native call occurs under these guards.
use super::PdfImportResources;
use crate::{
    data_interface::{
        DatabaseGuard,
        plugin_resources::{ColumnCodec, RowPolicy},
    },
    runtime_plugins::{
        literature_attachments::{self, AttachmentProgress, AttachmentResources, AttachmentSql},
        literature_catalog::{
            data::CatalogResources,
            save::{
                self, CommittedCatalogRows, SaveCause, SaveFailure, SaveOutcome, SaveResources,
            },
        },
        literature_import_jobs::{
            self as jobs, ImportJobsProgress, ImportJobsResources, SqlOutcome,
        },
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    catalog_save::{CatalogSaveOutcome, CatalogSaveRejection},
    data::DataValue,
    import_jobs::{ImportJobRowsV1, ImportJobsProjectionInputV1},
    pdf_import::PreparedPdfImportEntryV1,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

mod header;

#[derive(Clone)]
pub(crate) struct PdfStorage {
    pub(crate) store: AppStore,
    pub(crate) pdf: PdfImportResources,
    pub(crate) jobs: ImportJobsResources,
    pub(crate) reads: CatalogResources,
    pub(crate) writes: SaveResources,
    pub(crate) attachments: AttachmentResources,
}
#[derive(Debug)]
pub(crate) struct PendingPdfResult {
    pub(crate) id: String,
    pub(crate) raw_input: String,
}
pub(crate) enum SaveStage {
    Saved(CommittedCatalogRows),
    Failed(String),
}

pub(crate) fn progress(job: &str, phase: &'static str) -> ImportJobsProgress {
    let mut progress = ImportJobsProgress::new(phase);
    progress.job_id = Some(job.into());
    progress.header_committed = Some(true);
    progress
}
pub(crate) fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}
fn nullable(value: Option<&str>) -> DataValue {
    value.map_or(DataValue::Null, text)
}
fn with_db<T>(
    storage: &PdfStorage,
    progress: &ImportJobsProgress,
    work: impl FnOnce(&mut DatabaseGuard<'_>) -> NativeResult<T>,
) -> NativeResult<T> {
    let mut db = storage
        .store
        .connection()
        .map_err(|cause| progress.bind(AttachmentProgress::new("owner").owner(cause).native()))?;
    match catch_unwind(AssertUnwindSafe(|| work(&mut db))) {
        Ok(result) => result,
        Err(payload) => {
            db.require_recovery();
            std::panic::resume_unwind(payload)
        }
    }
}

pub(crate) fn create(
    storage: &PdfStorage,
    entries: &[PreparedPdfImportEntryV1],
) -> NativeResult<(String, ImportJobRowsV1, ImportJobsProgress)> {
    let mut p = ImportJobsProgress::new("pdf-header");
    let id = uuid::Uuid::new_v4().to_string();
    p.job_id = Some(id.clone());
    with_db(storage, &p.clone(), |db| {
        header::create(db, storage, &id, entries, &mut p)?;
        p.header_committed = Some(true);
        p.phase = "pdf-header-read";
        let rows = jobs::data::job(db, &storage.jobs, &id, &p)?;
        Ok((id, rows, p))
    })
}

pub(crate) fn mark_running(storage: &PdfStorage, job: &str) -> NativeResult<()> {
    let mut p = progress(job, "pdf-running");
    with_db(storage, &p.clone(), |db| {
        p.outcome = SqlOutcome::StatementFailed;
        storage
            .pdf
            .write(db, "mark-running", vec![text(job)])
            .map_err(|cause| p.data(cause))?;
        Ok(())
    })
}
pub(crate) fn pending(storage: &PdfStorage, job: &str) -> NativeResult<Vec<PendingPdfResult>> {
    let p = progress(job, "pdf-pending");
    with_db(storage, &p, |db| {
        let rows = storage
            .pdf
            .read(
                db,
                "pending",
                vec![text(job)],
                RowPolicy::All,
                &[ColumnCodec::String; 2],
            )
            .map_err(|cause| p.data(cause))?;
        rows.rows
            .into_iter()
            .map(|row| match row.as_slice() {
                [DataValue::Text(id), DataValue::Text(raw)] => Ok(PendingPdfResult {
                    id: id.clone(),
                    raw_input: raw.clone(),
                }),
                _ => Err(p.mismatch()),
            })
            .collect()
    })
}

/// This is the original single observation at the start of an item. Later
/// cancellation is consumed by the next item, not rechecked after Native work.
pub(crate) fn observe_cancel(storage: &PdfStorage, job: &str) -> NativeResult<bool> {
    let mut p = progress(job, "pdf-cancel-check");
    with_db(storage, &p.clone(), |db| {
        let rows = storage
            .pdf
            .read(
                db,
                "cancel-requested",
                vec![text(job)],
                RowPolicy::FirstRequired,
                &[ColumnCodec::Bool],
            )
            .map_err(|cause| p.data(cause))?;
        let [DataValue::Integer(value)] = rows.rows.first().map(Vec::as_slice).unwrap_or(&[])
        else {
            return Err(p.mismatch());
        };
        if *value == 0 {
            return Ok(false);
        }
        p.phase = "pdf-cancel-pending";
        p.outcome = SqlOutcome::StatementFailed;
        storage
            .pdf
            .write(db, "cancel-pending", vec![text(job)])
            .map_err(|cause| p.data(cause))?;
        p.phase = "pdf-cancel-job";
        storage
            .pdf
            .write(db, "cancel-job", vec![text(job)])
            .map_err(|cause| p.data(cause))?;
        Ok(true)
    })
}

pub(crate) fn save(
    storage: &PdfStorage,
    outcome: CatalogSaveOutcome,
    p: &mut ImportJobsProgress,
) -> NativeResult<SaveStage> {
    with_db(storage, &p.clone(), |db| match outcome {
        CatalogSaveOutcome::Rejected { reason } => {
            // Original save began Deferred before validating title/type.
            if let Err(failure) = save::probe_begin(db) {
                return save_failure(failure, p);
            }
            p.last_save = Some(SaveOutcome::RolledBack);
            Ok(SaveStage::Failed(match reason {
                CatalogSaveRejection::EmptyTitle => "标题不能为空".into(),
                CatalogSaveRejection::UnsupportedType { item_type } => {
                    format!("不支持的文献类型：{item_type}")
                }
            }))
        }
        CatalogSaveOutcome::Planned { plan } => {
            match save::persist(db, &storage.writes, &storage.reads, &plan) {
                Ok(rows) => {
                    p.last_save = Some(SaveOutcome::Committed {
                        item_id: rows.item_id.clone(),
                    });
                    Ok(SaveStage::Saved(rows))
                }
                Err(failure) => save_failure(failure, p),
            }
        }
    })
}
fn save_failure(failure: SaveFailure, p: &mut ImportJobsProgress) -> NativeResult<SaveStage> {
    p.last_save = Some(failure.outcome.clone());
    let original = match &failure.cause {
        SaveCause::Data(cause) => cause.original_sql_failure.as_ref(),
        SaveCause::Database(cause) => cause.original_sql_failure.as_ref(),
    };
    if failure.outcome != SaveOutcome::Unknown
        && let Some(original) = original
    {
        return Ok(SaveStage::Failed(format!(
            "文献数据库操作失败：{}",
            original.message()
        )));
    }
    p.outcome = if failure.outcome == SaveOutcome::Unknown {
        SqlOutcome::Unknown
    } else {
        SqlOutcome::NotStarted
    };
    let mut error = NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "storage_error",
        format!("文献数据库操作失败：{failure}"),
    );
    error.save_outcome = Some(failure.outcome);
    error.save_rollback_error = failure.rollback_error;
    match failure.cause {
        SaveCause::Data(cause) => error.data_error = Some(cause),
        SaveCause::Database(cause) => error.save_database_error = Some(cause),
    }
    Err(p.bind(error))
}

/// Only an attachment failure enters the original parent DELETE. An earlier
/// committed-save readback failure is handled before calling this function.
pub(crate) fn attach(
    storage: &PdfStorage,
    item: &str,
    path: &str,
    p: &mut ImportJobsProgress,
) -> NativeResult<Result<String, String>> {
    with_db(
        storage,
        &p.clone(),
        |db| match literature_attachments::attach(
            db,
            &storage.store,
            &storage.attachments,
            item,
            path,
        ) {
            Ok(_) => Ok(Ok(item.into())),
            Err(failure) => {
                if failure.progress.sql == AttachmentSql::Unknown {
                    return Err(p.bind(failure.native()));
                }
                let message = failure
                    .data
                    .as_ref()
                    .and_then(|cause| cause.original_sql_failure.as_ref())
                    .or_else(|| {
                        failure
                            .database
                            .as_ref()
                            .and_then(|cause| cause.original_sql_failure.as_ref())
                    })
                    .map_or_else(
                        || failure.message.clone(),
                        |original| format!("文献数据库操作失败：{}", original.message()),
                    );
                p.phase = "pdf-delete-failed-item";
                // A failed DELETE supersedes the attachment failure, as before.
                match storage
                    .pdf
                    .write(db, "delete-failed-item", vec![text(item)])
                {
                    Ok(_) => Ok(Err(message)),
                    Err(cause) => match cause.original_sql_failure.as_ref() {
                        Some(original) => {
                            Ok(Err(format!("文献数据库操作失败：{}", original.message())))
                        }
                        None => Err(p.data(cause)),
                    },
                }
            }
        },
    )
}
pub(crate) fn record(
    storage: &PdfStorage,
    result: &str,
    outcome: Result<String, String>,
    index: usize,
    p: &mut ImportJobsProgress,
) -> NativeResult<()> {
    let (state, item, message) = match outcome {
        Ok(item) => ("imported", Some(item), None),
        Err(message) => ("failed", None, Some(message)),
    };
    with_db(storage, &p.clone(), |db| {
        p.phase = "pdf-result-update";
        p.outcome = SqlOutcome::StatementFailed;
        storage
            .jobs
            .write_on_connection(
                db,
                "result",
                vec![
                    text(result),
                    text(state),
                    nullable(item.as_deref()),
                    nullable(message.as_deref()),
                ],
            )
            .map_err(|cause| p.data(cause))?;
        p.outcome = SqlOutcome::Returned;
        p.completed_result_updates = Some(index + 1);
        p.phase = "pdf-progress";
        p.outcome = SqlOutcome::StatementFailed;
        let job = p.job_id.as_deref().ok_or_else(|| p.mismatch())?;
        storage
            .pdf
            .write(db, "progress", vec![text(job)])
            .map_err(|cause| p.data(cause))?;
        p.outcome = SqlOutcome::Returned;
        Ok(())
    })
}
pub(crate) fn finish(storage: &PdfStorage, job: &str) -> NativeResult<()> {
    let mut p = progress(job, "pdf-aggregate");
    with_db(storage, &p.clone(), |db| {
        let rows = storage
            .jobs
            .read(
                db,
                "counts",
                vec![text(job)],
                RowPolicy::FirstRequired,
                &[ColumnCodec::Usize; 2],
            )
            .map_err(|cause| p.data(cause))?;
        let [DataValue::Integer(imported), DataValue::Integer(failed)] =
            rows.rows.first().map(Vec::as_slice).unwrap_or(&[])
        else {
            return Err(p.mismatch());
        };
        let state = if *failed == 0 {
            "completed"
        } else if *imported == 0 {
            "failed"
        } else {
            "completed_with_errors"
        };
        p.phase = "pdf-finish";
        p.outcome = SqlOutcome::StatementFailed;
        storage
            .jobs
            .write_on_connection(db, "finish", vec![text(job), text(state)])
            .map_err(|cause| p.data(cause))?;
        Ok(())
    })
}

/// Both original best-effort writes are attempted even when the first fails.
/// Original SQL text is used only as persisted business content, never copied
/// into the returned/serialized Native diagnostic.
pub(crate) fn fail(storage: &PdfStorage, job: &str, failure: &NativeHostError) -> NativeResult<()> {
    let p = progress(job, "pdf-fail");
    let original = failure
        .data_error
        .as_ref()
        .and_then(|cause| cause.original_sql_failure.as_ref())
        .or_else(|| {
            failure
                .save_database_error
                .as_ref()
                .and_then(|cause| cause.original_sql_failure.as_ref())
        });
    let message = original.map_or_else(
        || failure.message.clone(),
        |original| format!("文献数据库操作失败：{}", original.message()),
    );
    with_db(storage, &p, |db| {
        let first = storage.pdf.write(db, "fail-job", vec![text(job)]);
        let second = storage
            .pdf
            .write(db, "fail-pending", vec![text(job), text(&message)]);
        first.map_err(|cause| p.data(cause))?;
        second.map_err(|cause| p.data(cause))?;
        Ok(())
    })
}

pub(crate) fn header_input(id: String, rows: ImportJobRowsV1) -> ImportJobsProjectionInputV1 {
    ImportJobsProjectionInputV1::Job {
        expected_id: id,
        rows,
    }
}
