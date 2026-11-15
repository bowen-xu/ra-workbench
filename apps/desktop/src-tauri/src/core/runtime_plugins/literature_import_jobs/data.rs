use super::{ImportJobsProgress, ImportJobsResources};
use crate::{
    data_interface::plugin_resources::{ColumnCodec, RowPolicy},
    runtime_plugins::native::NativeResult,
};
use research_workbench_native_sdk::{
    data::DataValue,
    import_jobs::{ImportJobRowsV1, ImportJobsProjectionInputV1},
};
use rusqlite::Connection;

pub(crate) fn job(
    db: &Connection,
    resources: &ImportJobsResources,
    id: &str,
    progress: &ImportJobsProgress,
) -> NativeResult<ImportJobRowsV1> {
    let header = resources
        .read(
            db,
            "header",
            vec![DataValue::Text(id.into())],
            RowPolicy::FirstOptional,
            &[
                ColumnCodec::String,
                ColumnCodec::String,
                ColumnCodec::String,
                ColumnCodec::Usize,
                ColumnCodec::Usize,
                ColumnCodec::Bool,
                ColumnCodec::String,
                ColumnCodec::String,
            ],
        )
        .map_err(|cause| progress.data(cause))?;
    let results = if header.rows.is_empty() {
        None
    } else {
        Some(
            resources
                .read(
                    db,
                    "results",
                    vec![DataValue::Text(id.into())],
                    RowPolicy::All,
                    &[
                        ColumnCodec::String,
                        ColumnCodec::String,
                        ColumnCodec::String,
                        ColumnCodec::OptionalString,
                        ColumnCodec::OptionalString,
                    ],
                )
                .map_err(|cause| progress.data(cause))?,
        )
    };
    Ok(ImportJobRowsV1 { header, results })
}

pub(crate) fn list(
    db: &Connection,
    resources: &ImportJobsResources,
    progress: &ImportJobsProgress,
) -> NativeResult<ImportJobsProjectionInputV1> {
    // Collect every original selector ID before the first full job read.
    let ids = resources
        .read(
            db,
            "list-ids",
            vec![],
            RowPolicy::All,
            &[ColumnCodec::String],
        )
        .map_err(|cause| progress.data(cause))?;
    let expected_ids = ids
        .rows
        .into_iter()
        .map(|row| match row.into_iter().next() {
            Some(DataValue::Text(id)) => Ok(id),
            _ => Err(progress.mismatch()),
        })
        .collect::<NativeResult<Vec<_>>>()?;
    let mut rows = Vec::with_capacity(expected_ids.len());
    for id in &expected_ids {
        let captured = job(db, resources, id, progress)?;
        let missing = captured.header.rows.is_empty();
        rows.push(captured);
        if missing {
            break;
        }
    }
    Ok(ImportJobsProjectionInputV1::Jobs { expected_ids, rows })
}
