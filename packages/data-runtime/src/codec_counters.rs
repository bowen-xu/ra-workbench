//! Optional observation of actual coordinator codec calls, never a second codec pass.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Counts successful codec operations and the bytes they actually produced or
/// consumed. Record decoding includes its canonical command re-encoding and
/// fingerprint validation; the categories therefore overlap by design.
/// These are logical JSON bytes, not allocation size or filesystem I/O.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecSnapshot {
    pub command_encodes: u64,
    pub command_encoded_bytes: u64,
    pub command_round_trips: u64,
    pub command_round_trip_bytes: u64,
    pub record_encodes: u64,
    pub record_encoded_bytes: u64,
    pub record_decodes: u64,
    pub record_decoded_bytes: u64,
    pub fingerprints: u64,
    pub fingerprint_bytes: u64,
}

#[derive(Debug, Default)]
struct Counters {
    command_encodes: AtomicU64,
    command_encoded_bytes: AtomicU64,
    command_round_trips: AtomicU64,
    command_round_trip_bytes: AtomicU64,
    record_encodes: AtomicU64,
    record_encoded_bytes: AtomicU64,
    record_decodes: AtomicU64,
    record_decoded_bytes: AtomicU64,
    fingerprints: AtomicU64,
    fingerprint_bytes: AtomicU64,
}

/// Per-owner optional counters; clones observe the same opening, including
/// recovery and post-close sampling. Snapshots are consistent only at quiescent
/// boundaries. Counters do not own an engine, file, command or payload.
#[derive(Clone, Debug, Default)]
pub struct CodecCounters(Arc<Counters>);

impl CodecCounters {
    pub fn snapshot(&self) -> CodecSnapshot {
        let c = &self.0;
        let read = |v: &AtomicU64| v.load(Ordering::Relaxed);
        CodecSnapshot {
            command_encodes: read(&c.command_encodes),
            command_encoded_bytes: read(&c.command_encoded_bytes),
            command_round_trips: read(&c.command_round_trips),
            command_round_trip_bytes: read(&c.command_round_trip_bytes),
            record_encodes: read(&c.record_encodes),
            record_encoded_bytes: read(&c.record_encoded_bytes),
            record_decodes: read(&c.record_decodes),
            record_decoded_bytes: read(&c.record_decoded_bytes),
            fingerprints: read(&c.fingerprints),
            fingerprint_bytes: read(&c.fingerprint_bytes),
        }
    }
    fn add(count: &AtomicU64, bytes: &AtomicU64, len: usize) {
        count.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(len as u64, Ordering::Relaxed);
    }
    pub(crate) fn command_encoded(&self, n: usize) {
        Self::add(&self.0.command_encodes, &self.0.command_encoded_bytes, n);
    }
    pub(crate) fn command_round_trip(&self, n: usize) {
        Self::add(
            &self.0.command_round_trips,
            &self.0.command_round_trip_bytes,
            n,
        );
    }
    pub(crate) fn record_encoded(&self, n: usize) {
        Self::add(&self.0.record_encodes, &self.0.record_encoded_bytes, n);
    }
    pub(crate) fn record_decoded(&self, n: usize) {
        Self::add(&self.0.record_decodes, &self.0.record_decoded_bytes, n);
    }
    pub(crate) fn fingerprint(&self, n: usize) {
        Self::add(&self.0.fingerprints, &self.0.fingerprint_bytes, n);
    }
}
