//! The payload codec is independent of Raft Engine's protobuf/index envelope.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored<T> {
    schema_version: u32,
    record: T,
}

pub(super) fn command_bytes<C: Serialize>(
    command: &C,
    position: u64,
    counters: Option<&CodecCounters>,
) -> CoordinatorResult<Vec<u8>> {
    let mut value = serde_json::to_value(command).map_err(|_| codec(position, "command_encode"))?;
    sort_objects(&mut value);
    let bytes = serde_json::to_vec(&value).map_err(|_| codec(position, "command_encode"))?;
    if let Some(counters) = counters {
        counters.command_encoded(bytes.len());
    }
    Ok(bytes)
}
fn sort_objects(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.values_mut().for_each(sort_objects);
            map.sort_keys();
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(sort_objects),
        _ => {}
    }
}
pub(super) fn fingerprint(bytes: &[u8], counters: Option<&CodecCounters>) -> Fingerprint {
    if let Some(counters) = counters {
        counters.fingerprint(bytes.len());
    }
    Sha256::digest(bytes).into()
}
pub(super) fn encode<C: Serialize, R: Serialize>(
    record: &Accepted<C, R>,
    counters: Option<&CodecCounters>,
) -> CoordinatorResult<Vec<u8>> {
    let bytes = serde_json::to_vec(&Stored {
        schema_version: 1,
        record,
    })
    .map_err(|_| codec(record.position, "record_encode"))?;
    if let Some(counters) = counters {
        counters.record_encoded(bytes.len());
    }
    Ok(bytes)
}
pub(super) fn decode<P: Projector>(
    bytes: &[u8],
    position: u64,
    epoch: Uuid,
    max_command: usize,
    counters: Option<&CodecCounters>,
) -> CoordinatorResult<Accepted<P::Command, P::Receipt>> {
    let stored: Stored<Accepted<P::Command, P::Receipt>> =
        serde_json::from_slice(bytes).map_err(|_| codec(position, "record_decode"))?;
    if stored.schema_version != 1 {
        return Err(codec(position, "unknown_schema_version"));
    }
    let record = stored.record;
    if record.epoch != epoch {
        return Err(CoordinatorError::EpochMismatch);
    }
    if record.position != position
        || record.operation_id.is_nil()
        || P::operation_id(&record.command) != record.operation_id
    {
        return Err(CoordinatorError::Corrupt {
            position,
            reason: "record_identity",
        });
    }
    let command = command_bytes(&record.command, position, counters)?;
    if command.len() > max_command {
        return Err(codec(position, "command_budget"));
    }
    if fingerprint(&command, counters) != record.fingerprint {
        return Err(CoordinatorError::Corrupt {
            position,
            reason: "command_fingerprint",
        });
    }
    if let Some(counters) = counters {
        counters.record_decoded(bytes.len());
    }
    Ok(record)
}
pub(super) fn codec(position: u64, reason: &'static str) -> CoordinatorError {
    CoordinatorError::Codec { position, reason }
}
