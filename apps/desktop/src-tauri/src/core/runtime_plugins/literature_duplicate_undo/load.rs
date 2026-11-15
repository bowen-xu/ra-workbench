use super::{UndoCause, UndoPhase as Phase, UndoProblem, UndoResources};
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P, read_on_connection},
    runtime_plugins::{
        data::AcceptedDataResource,
        literature_nal_bootstrap::{CompleteNalLoadError, NalLoadResources, load_complete},
    },
};
use research_workbench_native_sdk::{
    data::{DataCall, DataRows, DataValue},
    duplicate_merge::{CollectionAssignment, CollectionMembership, DuplicateMergeSnapshot},
    nal_annotation::NalAnnotation,
    nal_state::NalStatement,
};
use rusqlite::Connection;
use serde::{Serialize, de::DeserializeOwned};

pub(super) type Result<T> = std::result::Result<T, UndoProblem>;

pub(crate) struct CapturedUndo {
    pub(super) operation_id: String,
    pub(super) survivor_item_id: String,
    pub(super) merged_item_id: String,
    pub(crate) snapshot_json: String,
    pub(super) snapshot: DuplicateMergeSnapshot,
}

/// The original optional header and snapshot decode both precede BEGIN.
pub(crate) fn capture(
    db: &Connection,
    resources: &UndoResources,
    operation_id: String,
) -> Result<CapturedUndo> {
    let rows = read(
        db,
        &resources.header,
        vec![text(&operation_id)],
        Phase::Header,
        P::FirstOptional,
        &[C::String; 3],
    )?;
    let Some(row) = rows.rows.first() else {
        return Err(UndoProblem {
            phase: Phase::Header,
            cause: UndoCause::Missing,
        });
    };
    let [
        DataValue::Text(survivor),
        DataValue::Text(merged),
        DataValue::Text(snapshot_json),
    ] = row.as_slice()
    else {
        return Err(UndoProblem::binding(Phase::Header, "typed-header"));
    };
    let snapshot = serde_json::from_str(snapshot_json).map_err(|error| UndoProblem {
        phase: Phase::Snapshot,
        cause: UndoCause::SnapshotCodec(error.to_string()),
    })?;
    Ok(CapturedUndo {
        operation_id,
        survivor_item_id: survivor.clone(),
        merged_item_id: merged.clone(),
        snapshot_json: snapshot_json.clone(),
        snapshot,
    })
}

pub(super) fn read(
    db: &Connection,
    resource: &AcceptedDataResource,
    arguments: Vec<DataValue>,
    phase: Phase,
    policy: P,
    codecs: &[C],
) -> Result<DataRows> {
    read_on_connection(db, resource, &call(resource, arguments), policy, codecs)
        .map_err(|cause| UndoProblem::data(phase, cause))
}

pub(super) fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}
pub(super) fn text(value: &str) -> DataValue {
    DataValue::Text(value.to_owned())
}

pub(super) fn encode<T: Serialize + ?Sized>(value: &T, phase: Phase) -> Result<String> {
    serde_json::to_string(value)
        .map_err(|error| UndoProblem::binding_codec(phase, "sdk-codec", &error))
}

fn pair_rows<T>(
    rows: DataRows,
    phase: Phase,
    make: impl Fn(String, String) -> T,
) -> Result<Vec<T>> {
    rows.rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [DataValue::Text(a), DataValue::Text(b)] => Ok(make(a.clone(), b.clone())),
            _ => Err(UndoProblem::binding(phase, "typed-pair-row")),
        })
        .collect()
}

/// The six original queries stay complete, including unused typed columns.
pub(super) fn classifications(
    db: &Connection,
    r: &UndoResources,
) -> Result<(Vec<CollectionMembership>, Vec<CollectionAssignment>)> {
    let phase = Phase::ClassificationLoad;
    read(
        db,
        &r.collection_revision,
        vec![],
        phase,
        P::FirstRequired,
        &[C::I64],
    )?;
    read(
        db,
        &r.collection_collections,
        vec![],
        phase,
        P::All,
        &[C::String, C::String, C::String, C::OptionalString],
    )?;
    read(
        db,
        &r.collection_types,
        vec![],
        phase,
        P::All,
        &[C::String, C::String, C::String, C::Bool, C::Bool],
    )?;
    read(
        db,
        &r.collection_relations,
        vec![],
        phase,
        P::All,
        &[C::String; 4],
    )?;
    let memberships = pair_rows(
        read(
            db,
            &r.collection_memberships,
            vec![],
            phase,
            P::All,
            &[C::String; 2],
        )?,
        phase,
        |item_id, collection_id| CollectionMembership {
            item_id,
            collection_id,
        },
    )?;
    let assignments = pair_rows(
        read(
            db,
            &r.collection_assignments,
            vec![],
            phase,
            P::All,
            &[C::String; 2],
        )?,
        phase,
        |item_id, relation_id| CollectionAssignment {
            item_id,
            relation_id,
        },
    )?;
    Ok((memberships, assignments))
}

pub(super) fn nal(
    db: &Connection,
    r: &UndoResources,
) -> Result<(Vec<NalStatement>, Vec<NalAnnotation>)> {
    let resources = NalLoadResources {
        revision: &r.nal_revision,
        concepts: &r.nal_concepts,
        statements: &r.nal_statements,
        annotations: &r.nal_annotations,
        rules: &r.nal_rules,
    };
    let state = load_complete(resources, |_, resource, policy, codecs| {
        read(db, resource, vec![], Phase::NalLoad, policy, codecs)
    })
    .map_err(|error| match error {
        CompleteNalLoadError::Read(error) => error,
        CompleteNalLoadError::Capture { .. } => UndoProblem {
            phase: Phase::NalLoad,
            cause: UndoCause::Structure,
        },
        CompleteNalLoadError::Structure(error) => {
            UndoProblem::structure_codec(Phase::NalLoad, &error)
        }
    })?;
    // Rule strings, revision and concepts were read/decoded at the original
    // points. Undo never consumes their pure projected presentation values.
    Ok((state.statements, state.raw.annotations))
}

pub(super) fn scope<T: DeserializeOwned>(
    db: &Connection,
    resource: &AcceptedDataResource,
    a: &str,
    b: &str,
    phase: Phase,
) -> Result<Vec<T>> {
    let rows = read(
        db,
        resource,
        vec![text(a), text(b)],
        phase,
        P::All,
        &[C::String],
    )?;
    rows.rows
        .into_iter()
        .map(|row| {
            let [DataValue::Text(json)] = row.as_slice() else {
                return Err(UndoProblem::binding(phase, "scope-row"));
            };
            // Only fixed scope SELECTs over this transaction's already
            // SDK-decoded DTOs reach here. Their extra row envelope must not
            // lower the original bare-Term recursion budget. External and
            // stored-input decoders above keep their original limit.
            let mut decoder = serde_json::Deserializer::from_str(json);
            decoder.disable_recursion_limit();
            let value = T::deserialize(&mut decoder)
                .map_err(|error| UndoProblem::binding_codec(phase, "scope-codec", &error))?;
            decoder
                .end()
                .map_err(|error| UndoProblem::binding_codec(phase, "scope-tail", &error))?;
            Ok(value)
        })
        .collect()
}

pub(super) fn equal<T: Serialize>(
    db: &Connection,
    r: &UndoResources,
    actual: &[T],
    expected: &str,
    phase: Phase,
) -> Result<bool> {
    let actual = encode(actual, phase)?;
    let rows = read(
        db,
        &r.vector_equal,
        vec![text(&actual), text(expected)],
        phase,
        P::FirstRequired,
        &[C::Bool],
    )?;
    match rows.rows.first().map(Vec::as_slice) {
        Some([DataValue::Integer(value)]) => Ok(*value != 0),
        _ => Err(UndoProblem::binding(phase, "equality-row")),
    }
}
