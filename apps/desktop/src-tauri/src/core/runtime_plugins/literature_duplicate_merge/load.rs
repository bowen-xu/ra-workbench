use super::{MergeCause, MergePhase as Phase, MergeProblem, MergeResources};
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P, read_on_connection},
    runtime_plugins::data::AcceptedDataResource,
};
use research_workbench_native_sdk::{
    data::{DataCall, DataRows, DataValue as V},
    duplicate_merge_prepare::MergePrepareInputV1,
};
use rusqlite::Connection;
use serde::{Serialize, de::DeserializeOwned};

pub(crate) struct CapturedMerge {
    pub(crate) input: MergePrepareInputV1,
}
pub(super) type Result<T> = std::result::Result<T, MergeProblem>;

pub(crate) fn capture(
    db: &Connection,
    r: &MergeResources,
    candidate_id: String,
    survivor_item_id: String,
) -> Result<CapturedMerge> {
    let rows = read(
        db,
        &r.outer.header,
        vec![text(&candidate_id)],
        Phase::Header,
        P::FirstOptional,
        &[C::String; 2],
    )?;
    let Some(row) = rows.rows.first() else {
        return Err(MergeProblem {
            phase: Phase::Header,
            cause: MergeCause::Missing,
        });
    };
    let [V::Text(left), V::Text(right)] = row.as_slice() else {
        return Err(MergeProblem::binding(Phase::Header, "typed-candidate"));
    };
    Ok(CapturedMerge {
        input: MergePrepareInputV1 {
            candidate_id,
            survivor_item_id,
            left_item_id: left.clone(),
            right_item_id: right.clone(),
        },
    })
}

pub(super) fn call(resource: &AcceptedDataResource, arguments: Vec<V>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}
pub(super) fn read(
    db: &Connection,
    resource: &AcceptedDataResource,
    args: Vec<V>,
    phase: Phase,
    policy: P,
    codecs: &[C],
) -> Result<DataRows> {
    read_on_connection(db, resource, &call(resource, args), policy, codecs)
        .map_err(|e| MergeProblem::data(phase, e))
}
pub(super) fn text(value: &str) -> V {
    V::Text(value.to_owned())
}
pub(super) fn encode<T: Serialize + ?Sized>(value: &T, phase: Phase) -> Result<String> {
    serde_json::to_string(value).map_err(|e| MergeProblem {
        phase,
        cause: MergeCause::SnapshotCodec(e.to_string()),
    })
}
pub(super) fn strings(rows: DataRows, phase: Phase) -> Result<Vec<String>> {
    rows.rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [V::Text(value)] => Ok(value.clone()),
            _ => Err(MergeProblem::binding(phase, "typed-string-row")),
        })
        .collect()
}
pub(super) fn one(rows: DataRows, phase: Phase) -> Result<String> {
    strings(rows, phase)?
        .into_iter()
        .next()
        .ok_or_else(|| MergeProblem::binding(phase, "required-string-row"))
}
pub(super) fn scope<T: Serialize, U: DeserializeOwned>(
    db: &Connection,
    resource: &AcceptedDataResource,
    values: &T,
    id: &str,
    phase: Phase,
) -> Result<Vec<U>> {
    let json = encode(values, phase)?;
    let rows = read(
        db,
        resource,
        vec![V::Text(json), text(id)],
        phase,
        P::All,
        &[C::String],
    )?;
    strings(rows, phase)?
        .into_iter()
        .map(|json| {
            // Only the already typed canonical DTO envelope reaches this local
            // decoder. Stored/external bare-Term codecs retain their original limit.
            let mut decoder = serde_json::Deserializer::from_str(&json);
            decoder.disable_recursion_limit();
            let value = U::deserialize(&mut decoder)
                .map_err(|_| MergeProblem::binding(phase, "scope-codec"))?;
            decoder
                .end()
                .map_err(|_| MergeProblem::binding(phase, "scope-tail"))?;
            Ok(value)
        })
        .collect()
}
