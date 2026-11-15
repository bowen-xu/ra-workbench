//! Keep complete ID collection and each row/JSON codec in their original order.
use super::{ReferenceFailure, ReferenceProblem, ReferenceProgress, ReferenceResources};
use crate::data_interface::plugin_resources::{ColumnCodec, RowPolicy};
use research_workbench_native_sdk::{
    data::DataValue,
    reference_records::{ReferenceMetadataField, ReferenceRecord, ReferenceRecordRejectionV1},
};
use rusqlite::Connection;

type Result<T> = std::result::Result<T, ReferenceFailure>;

pub(crate) fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}

pub(crate) fn optional_text(value: Option<&str>) -> DataValue {
    value.map_or(DataValue::Null, text)
}

pub(crate) fn list(
    db: &Connection,
    resources: &ReferenceResources,
    paper_id: &str,
) -> Result<Vec<ReferenceRecord>> {
    let progress = ReferenceProgress::new("list-ids");
    let ids = resources
        .read(
            db,
            "ids",
            vec![text(paper_id)],
            RowPolicy::All,
            &[ColumnCodec::String],
        )
        .map_err(|error| progress.data(error))?;
    let ids = ids
        .rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [DataValue::Text(id)] => Ok(id.clone()),
            _ => Err(progress.data(resources.shape_error("ids"))),
        })
        .collect::<Result<Vec<_>>>()?;
    // Never batch the main rows: an earlier fields codec failure must precede
    // even a later row's SQL or primitive conversion failure.
    ids.iter()
        .map(|id| record(db, resources, id, &progress.at("list-record")))
        .collect()
}

pub(crate) fn record(
    db: &Connection,
    resources: &ReferenceResources,
    id: &str,
    progress: &ReferenceProgress,
) -> Result<ReferenceRecord> {
    let mut progress = progress.clone();
    progress.record_id = Some(id.into());
    let result = resources
        .record(db, id)
        .map_err(|error| progress.data(error))?;
    let row = result
        .rows
        .first()
        .ok_or_else(|| progress.failure(ReferenceProblem::Missing))?;
    let string = |index: usize| match row.get(index) {
        Some(DataValue::Text(value)) => Ok(value.clone()),
        _ => Err(progress.data(resources.shape_error("record"))),
    };
    let optional = |index: usize| match row.get(index) {
        Some(DataValue::Null) => Ok(None),
        Some(DataValue::Text(value)) => Ok(Some(value.clone())),
        _ => Err(progress.data(resources.shape_error("record"))),
    };
    let revision = match row.get(13) {
        Some(DataValue::Integer(value)) => *value,
        _ => return Err(progress.data(resources.shape_error("record"))),
    };
    let mut value = ReferenceRecord {
        id: string(0)?,
        citing_paper_id: string(1)?,
        source_anchor_id: string(2)?,
        raw_text: string(3)?,
        fields: Vec::new(),
        parse_state: string(5)?,
        identity_state: string(6)?,
        matched_item_id: optional(7)?,
        semantic_state: string(8)?,
        usage_kind: optional(9)?,
        usage_summary: string(10)?,
        usage_anchor_id: optional(11)?,
        provenance_kind: string(12)?,
        revision,
        created_at: string(14)?,
        updated_at: string(15)?,
    };
    value.fields =
        serde_json::from_str::<Vec<ReferenceMetadataField>>(&string(4)?).map_err(|_| {
            progress
                .at("fields-codec")
                .failure(ReferenceProblem::Domain(
                    ReferenceRecordRejectionV1::StoredFieldsEncoding,
                ))
        })?;
    Ok(value)
}

pub(super) fn belongs(
    db: &Connection,
    resources: &ReferenceResources,
    anchor_id: &str,
    paper_id: &str,
    progress: &ReferenceProgress,
) -> Result<bool> {
    scalar_bool(
        db,
        resources,
        "anchor-belongs",
        vec![text(anchor_id), text(paper_id)],
        progress,
    )
}

pub(super) fn matched_exists(
    db: &Connection,
    resources: &ReferenceResources,
    item_id: &str,
    progress: &ReferenceProgress,
) -> Result<bool> {
    scalar_bool(
        db,
        resources,
        "matched-exists",
        vec![text(item_id)],
        progress,
    )
}

fn scalar_bool(
    db: &Connection,
    resources: &ReferenceResources,
    key: &str,
    arguments: Vec<DataValue>,
    progress: &ReferenceProgress,
) -> Result<bool> {
    let rows = resources
        .read(
            db,
            key,
            arguments,
            RowPolicy::FirstRequired,
            &[ColumnCodec::Bool],
        )
        .map_err(|error| progress.data(error))?;
    match rows.rows.first().map(Vec::as_slice) {
        Some([DataValue::Integer(value)]) => Ok(*value != 0),
        _ => Err(progress.data(resources.shape_error(key))),
    }
}
