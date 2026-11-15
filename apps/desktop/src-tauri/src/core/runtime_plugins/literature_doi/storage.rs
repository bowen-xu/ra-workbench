//! Borrowed Core IO only: no HTTP, native call, outer transaction or domain policy.
use super::DoiResources;
use crate::{
    data_interface::{
        DatabaseGuard,
        plugin_resources::{ColumnCodec, RowPolicy, write_on_connection},
    },
    runtime_plugins::{
        data::AcceptedDataResource,
        literature_import_jobs::{
            ImportJobsProgress, ImportJobsResources, SqlOutcome, execute::create_header,
        },
        native::{NativeHostError, NativeProfile, NativeResult},
    },
};
use research_workbench_native_sdk::{
    data::{DataCall, DataValue},
    doi_metadata::MetadataPreviewV1,
    import_jobs::ImportFinishStatesV1,
};

pub(crate) fn create_job(
    db: &mut DatabaseGuard<'_>,
    jobs: &ImportJobsResources,
    source_kind: &str,
    normalized: &str,
    raw: &str,
) -> NativeResult<(String, String)> {
    let mut progress = ImportJobsProgress::new("header");
    let job_id = uuid::Uuid::new_v4().to_string();
    progress.job_id = Some(job_id.clone());
    let inputs = [(normalized.to_owned(), raw.to_owned())];
    let mut ids = create_header(db, jobs, &job_id, source_kind, &inputs, &mut progress)?;
    progress.header_committed = Some(true);
    let result_id = ids.pop().ok_or_else(|| progress.mismatch())?;
    Ok((job_id, result_id))
}

pub(crate) fn fail_job(
    db: &DatabaseGuard<'_>,
    jobs: &ImportJobsResources,
    doi: &DoiResources,
    job_id: &str,
    result_id: &str,
    message: &str,
    finish_states: &ImportFinishStatesV1,
) -> NativeResult<()> {
    let mut progress = progress("result-failed", job_id, result_id);
    if jobs.package_key() != doi.package_key() {
        return Err(progress.mismatch());
    }
    progress.outcome = SqlOutcome::StatementFailed;
    write(db, &doi.result_failed, vec![text(result_id), text(message)])
        .map_err(|cause| progress.data(cause))?;
    progress.completed_result_updates = Some(1);
    progress.outcome = SqlOutcome::Returned;
    progress.phase = "aggregate";
    let counts = jobs
        .read(
            db,
            "counts",
            vec![text(job_id)],
            RowPolicy::FirstRequired,
            &[ColumnCodec::Usize; 2],
        )
        .map_err(|cause| progress.data(cause))?;
    let [DataValue::Integer(imported), DataValue::Integer(failed)] =
        counts.rows.first().map(Vec::as_slice).unwrap_or(&[])
    else {
        return Err(progress.mismatch());
    };
    let state = if *failed == 0 {
        &finish_states.no_failed
    } else if *imported == 0 {
        &finish_states.no_imported
    } else {
        &finish_states.mixed
    };
    progress.phase = "finish-update";
    progress.outcome = SqlOutcome::StatementFailed;
    jobs.write_on_connection(db, "finish", vec![text(job_id), text(state)])
        .map_err(|cause| progress.data(cause))?;
    Ok(())
}

pub(crate) fn insert_candidate(
    db: &DatabaseGuard<'_>,
    doi: &DoiResources,
    preview: &MetadataPreviewV1,
    raw_payload: &str,
    result_id: &str,
) -> NativeResult<()> {
    let mut progress = progress("candidate-codec", &preview.import_job_id, result_id);
    progress.candidate_id = Some(preview.id.clone());
    progress.candidate_insert_returned = Some(false);
    let draft_json = serde_json::to_string(&preview.draft).map_err(|cause| {
        progress.bind(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "connector_error",
            format!("DOI 元数据请求失败：{cause}"),
        ))
    })?;
    progress.phase = "candidate-insert";
    progress.outcome = SqlOutcome::StatementFailed;
    // An unsuccessful autocommit statement can retain FAIL-trigger effects.
    progress.candidate_insert_returned = None;
    write(
        db,
        &doi.candidate_insert,
        vec![
            text(&preview.id),
            preview.item_id.as_deref().map_or(DataValue::Null, text),
            text(&preview.source_kind),
            text(&preview.source_reference),
            text(&draft_json),
            text(raw_payload),
            text(result_id),
        ],
    )
    .map_err(|cause| progress.data(cause))?;
    progress.candidate_insert_returned = Some(true);
    progress.phase = "queued-update";
    progress.outcome = SqlOutcome::StatementFailed;
    write(db, &doi.job_queued, vec![text(&preview.import_job_id)])
        .map_err(|cause| progress.data(cause))?;
    Ok(())
}

fn progress(phase: &'static str, job_id: &str, result_id: &str) -> ImportJobsProgress {
    let mut progress = ImportJobsProgress::new(phase);
    progress.job_id = Some(job_id.into());
    progress.result_id = Some(result_id.into());
    progress.header_committed = Some(true);
    progress
}

fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}

fn write(
    db: &DatabaseGuard<'_>,
    resource: &AcceptedDataResource,
    arguments: Vec<DataValue>,
) -> Result<u64, Box<crate::data_interface::plugin_resources::PluginDataError>> {
    write_on_connection(
        db,
        resource,
        &DataCall {
            id: resource.grant().id.clone(),
            version: resource.grant().version,
            arguments,
        },
    )
}
