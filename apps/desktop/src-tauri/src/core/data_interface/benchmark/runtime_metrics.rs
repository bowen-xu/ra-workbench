//! Per-log observation survives the owner; no global/path registry or extra I/O.
use super::{scenario::Mode, *};
use research_data_runtime::{CodecCounters, CodecSnapshot, LogIoCounters, LogIoSnapshot};

/// Delegator attempts and the sum of byte counts returned by successful writes.
/// Err may have written an unknown partial prefix; that byte count is NOT guessed.
/// Metadata/control files bypassing ExtentFileSystem are outside this counter.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct EngineIo {
    pub write_attempts: u64,
    pub reported_write_bytes: u64,
    pub write_errors: u64,
    pub sync_attempts: u64,
    pub sync_errors: u64,
}
impl From<LogIoSnapshot> for EngineIo {
    fn from(value: LogIoSnapshot) -> Self {
        Self {
            write_attempts: value.write_calls,
            reported_write_bytes: value.write_bytes,
            write_errors: value.write_errors,
            sync_attempts: value.sync_calls,
            sync_errors: value.sync_errors,
        }
    }
}
impl EngineIo {
    fn since(self, before: Self) -> Self {
        let delta = |after: u64, before: u64| {
            after
                .checked_sub(before)
                .expect("one per-log counter never resets")
        };
        Self {
            write_attempts: delta(self.write_attempts, before.write_attempts),
            reported_write_bytes: delta(self.reported_write_bytes, before.reported_write_bytes),
            write_errors: delta(self.write_errors, before.write_errors),
            sync_attempts: delta(self.sync_attempts, before.sync_attempts),
            sync_errors: delta(self.sync_errors, before.sync_errors),
        }
    }
}
pub(super) struct Observation(Option<(LogIoCounters, CodecCounters)>);
impl Observation {
    pub fn new(mode: Mode) -> Self {
        Self(
            mode.logged()
                .then(|| (LogIoCounters::default(), CodecCounters::default())),
        )
    }
    pub fn counters(&self) -> Option<LogIoCounters> {
        self.0.as_ref().map(|(io, _)| io.clone())
    }
    pub fn codec_counters(&self) -> Option<CodecCounters> {
        self.0.as_ref().map(|(_, codec)| codec.clone())
    }
    pub fn snapshot(&self) -> Option<EngineIo> {
        self.0
            .as_ref()
            .map(|(counters, _)| counters.snapshot().into())
    }
    pub fn delta(&self, before: Option<EngineIo>) -> Option<EngineIo> {
        self.snapshot()
            .zip(before)
            .map(|(after, before)| after.since(before))
    }
    pub fn codec_snapshot(&self) -> Option<CodecSnapshot> {
        self.0.as_ref().map(|(_, counters)| counters.snapshot())
    }
    pub fn codec_delta(&self, before: Option<CodecSnapshot>) -> Option<CodecSnapshot> {
        self.codec_snapshot().zip(before).map(|(after, before)| {
            let diff = |a: u64, b: u64| a.checked_sub(b).expect("one codec observer never resets");
            CodecSnapshot {
                command_encodes: diff(after.command_encodes, before.command_encodes),
                command_encoded_bytes: diff(
                    after.command_encoded_bytes,
                    before.command_encoded_bytes,
                ),
                command_round_trips: diff(after.command_round_trips, before.command_round_trips),
                command_round_trip_bytes: diff(
                    after.command_round_trip_bytes,
                    before.command_round_trip_bytes,
                ),
                record_encodes: diff(after.record_encodes, before.record_encodes),
                record_encoded_bytes: diff(after.record_encoded_bytes, before.record_encoded_bytes),
                record_decodes: diff(after.record_decodes, before.record_decodes),
                record_decoded_bytes: diff(after.record_decoded_bytes, before.record_decoded_bytes),
                fingerprints: diff(after.fingerprints, before.fingerprints),
                fingerprint_bytes: diff(after.fingerprint_bytes, before.fingerprint_bytes),
            }
        })
    }
}

#[test]
fn independent_log_observers_start_empty_and_delta_does_not_convert_absence_to_zero() {
    let legacy = Observation::new(Mode::LegacyFull);
    assert_eq!(legacy.snapshot(), None);
    assert_eq!(legacy.delta(None), None);
    assert_eq!(legacy.codec_snapshot(), None);
    assert_eq!(legacy.codec_delta(None), None);
    let one = Observation::new(Mode::AutoProcess);
    let two = Observation::new(Mode::AutoDevice);
    assert_eq!(one.snapshot(), Some(EngineIo::default()));
    assert_eq!(two.snapshot(), Some(EngineIo::default()));
    assert_eq!(one.delta(one.snapshot()), Some(EngineIo::default()));
    assert_eq!(one.codec_snapshot(), Some(CodecSnapshot::default()));
    assert_eq!(two.codec_snapshot(), Some(CodecSnapshot::default()));
    assert_eq!(
        one.codec_delta(one.codec_snapshot()),
        Some(CodecSnapshot::default())
    );
    let before = EngineIo {
        write_attempts: 2,
        reported_write_bytes: 10,
        write_errors: 1,
        sync_attempts: 3,
        sync_errors: 1,
    };
    let after = EngineIo {
        write_attempts: 7,
        reported_write_bytes: 31,
        write_errors: 2,
        sync_attempts: 8,
        sync_errors: 3,
    };
    assert_eq!(
        after.since(before),
        EngineIo {
            write_attempts: 5,
            reported_write_bytes: 21,
            write_errors: 1,
            sync_attempts: 5,
            sync_errors: 2
        }
    );
}
