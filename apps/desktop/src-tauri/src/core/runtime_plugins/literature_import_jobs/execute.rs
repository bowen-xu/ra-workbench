//! One original database guard spans the complete synchronous BibTeX batch.
use super::{ImportJobsProgress, ImportJobsResources, SqlOutcome, data};
use crate::{
    data_interface::{
        DatabaseGuard,
        plugin_resources::{ColumnCodec, RowPolicy},
    },
    runtime_plugins::{
        literature_catalog::{
            data::CatalogResources,
            save::{self, SaveCause, SaveFailure, SaveOutcome, SaveResources},
        },
        native::{NativeHostError, NativeProfile, NativeResult},
    },
};
use research_workbench_native_sdk::{
    catalog_save::{CatalogSaveOutcome, CatalogSaveRejection},
    data::DataValue,
    import_jobs::{ImportJobsProjectionInputV1, PreparedBibtexV1},
};

mod transaction;
pub(crate) use transaction::create_header;

fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}
fn nullable(value: Option<&str>) -> DataValue {
    value.map_or(DataValue::Null, text)
}

pub(crate) fn bibtex(
    db: &mut DatabaseGuard<'_>,
    jobs: &ImportJobsResources,
    catalog: &CatalogResources,
    writes: &SaveResources,
    plan: &PreparedBibtexV1,
) -> NativeResult<(ImportJobsProjectionInputV1, ImportJobsProgress)> {
    let mut progress = ImportJobsProgress::new("header");
    if jobs.package_key() != catalog.package_key() || jobs.package_key() != writes.package_key() {
        return Err(progress.mismatch());
    }
    let job_id = uuid::Uuid::new_v4().to_string();
    progress.job_id = Some(job_id.clone());
    let inputs = plan
        .entries
        .iter()
        .map(|entry| (entry.input_label.clone(), entry.raw_input.clone()))
        .collect::<Vec<_>>();
    let result_ids = create_header(
        db,
        jobs,
        &job_id,
        &plan.rules.source_kind,
        &inputs,
        &mut progress,
    )?;
    progress.header_committed = Some(true);
    for (index, (result_id, entry)) in result_ids.iter().zip(&plan.entries).enumerate() {
        progress.phase = "item-save";
        progress.result_id = Some(result_id.clone());
        progress.outcome = SqlOutcome::NotStarted;
        let item = match &entry.save {
            CatalogSaveOutcome::Rejected { reason } => {
                // The old save opens Deferred before rejecting a draft. Preserve
                // BEGIN failure precedence without writing a rejected entry.
                match save::probe_begin(db) {
                    Ok(()) => {
                        progress.last_save = Some(SaveOutcome::RolledBack);
                        Err(match reason {
                            CatalogSaveRejection::EmptyTitle => {
                                plan.rules.empty_title_message.clone()
                            }
                            CatalogSaveRejection::UnsupportedType { item_type } => {
                                format!("{}{item_type}", plan.rules.unsupported_type_prefix)
                            }
                        })
                    }
                    Err(failure) => {
                        item_failure(failure, &mut progress, &plan.rules.database_error_prefix)?
                    }
                }
            }
            CatalogSaveOutcome::Planned { plan: save_plan } => {
                match save::persist(db, writes, catalog, save_plan) {
                    Ok(committed) => {
                        progress.last_save = Some(SaveOutcome::Committed {
                            item_id: committed.item_id.clone(),
                        });
                        if committed
                            .items
                            .first()
                            .is_none_or(|item| item.main.rows.is_empty())
                        {
                            Err(plan.rules.not_found_message.clone())
                        } else {
                            Ok(committed.item_id)
                        }
                    }
                    Err(failure) => {
                        item_failure(failure, &mut progress, &plan.rules.database_error_prefix)?
                    }
                }
            }
        };
        let (state, item_id, message) = match item {
            Ok(id) => (&plan.rules.imported_state, Some(id), None),
            Err(message) => (&plan.rules.failed_state, None, Some(message)),
        };
        progress.phase = "result-update";
        progress.outcome = SqlOutcome::StatementFailed;
        jobs.write_on_connection(
            db,
            "result",
            vec![
                text(result_id),
                text(state),
                nullable(item_id.as_deref()),
                nullable(message.as_deref()),
            ],
        )
        .map_err(|cause| progress.data(cause))?;
        progress.outcome = SqlOutcome::Returned;
        progress.completed_result_updates = Some(index + 1);
    }
    progress.phase = "aggregate";
    progress.result_id = None;
    let counts = jobs
        .read(
            db,
            "counts",
            vec![text(&job_id)],
            RowPolicy::FirstRequired,
            &[ColumnCodec::Usize; 2],
        )
        .map_err(|cause| progress.data(cause))?;
    let [DataValue::Integer(imported), DataValue::Integer(failed)] =
        counts.rows.first().map(Vec::as_slice).unwrap_or(&[])
    else {
        return Err(progress.mismatch());
    };
    // The plugin supplies the three named outcomes. Core only branches on the
    // original two typed aggregate values, preserving failed-first precedence.
    let state = if *failed == 0 {
        &plan.rules.finish_states.no_failed
    } else if *imported == 0 {
        &plan.rules.finish_states.no_imported
    } else {
        &plan.rules.finish_states.mixed
    };
    progress.phase = "finish-update";
    progress.outcome = SqlOutcome::StatementFailed;
    jobs.write_on_connection(db, "finish", vec![text(&job_id), text(state)])
        .map_err(|cause| progress.data(cause))?;
    progress.outcome = SqlOutcome::Returned;
    progress.phase = "job-read";
    let rows = data::job(db, jobs, &job_id, &progress)?;
    Ok((
        ImportJobsProjectionInputV1::Job {
            expected_id: job_id,
            rows,
        },
        progress,
    ))
}

/// Ordinary save failures become a persisted per-entry outcome. An uncertain
/// transaction has already fenced the owner and must stop the batch here.
fn item_failure(
    failure: SaveFailure,
    progress: &mut ImportJobsProgress,
    prefix: &str,
) -> NativeResult<Result<String, String>> {
    progress.last_save = Some(failure.outcome.clone());
    let original = match &failure.cause {
        SaveCause::Data(cause) => cause.original_sql_failure.as_ref(),
        SaveCause::Database(cause) => cause.original_sql_failure.as_ref(),
    };
    if failure.outcome != SaveOutcome::Unknown
        && let Some(original) = original
    {
        return Ok(Err(format!("{prefix}{}", original.message())));
    }
    progress.outcome = if failure.outcome == SaveOutcome::Unknown {
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
    Err(progress.bind(error))
}
