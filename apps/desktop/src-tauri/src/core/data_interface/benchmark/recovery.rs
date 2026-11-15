//! The entire first-open recovery belongs to a fresh, independently sampled child.
use super::runtime_metrics::{EngineIo, Observation};
use super::{metrics::*, scenario::*, verification::*, *};
use research_data_runtime::CodecSnapshot;
use std::{path::Path, time::Instant};
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Recovery {
    pub mode: Mode,
    pub abrupt_exit: bool,
    pub window: WindowMeasurement,
    pub engine_io: Option<EngineIo>,
    pub pair_open_codec: Option<CodecSnapshot>,
    pub full_recovery_codec: Option<CodecSnapshot>,
    pub synthetic_ledger_and_expected_data_ms: f64,
    pub sql_events_at_first_open: u64,
    pub sql_prefix_read_ms: f64,
    pub pair_open_and_replay_ms: f64,
    pub verification_and_original_replay_ms: f64,
    pub sync_close_ms: f64,
    pub acknowledged_commands: usize,
    pub facts: Facts,
    pub first_open_cache_pages_written: u64,
    pub paired_cache_pages_written: u64,
    pub power_loss_tested: bool,
}
pub(super) fn run(path: &Path, abrupt_exit: bool) -> Recovery {
    // First statement: no SQLite/Raft handle, ledger parse or workload build has
    // happened in this process before its time/CPU/OS I/O/RSS baseline.
    let mut window = Window::start();
    let preparation = Instant::now();
    assert_fixture(path);
    let ledger: Ledger =
        serde_json::from_slice(&std::fs::read(path.join("acknowledged.json")).unwrap()).unwrap();
    let observation = Observation::new(ledger.job.mode);
    let io_before = observation.snapshot();
    let codec_before = observation.codec_snapshot();
    let data = dataset(&ledger.job);
    let preparation_ms = preparation.elapsed().as_secs_f64() * 1000.;
    let before = Instant::now();
    let sql = workload::connection(path, ledger.job.mode);
    let prefix = sql
        .query_row(
            "select coalesce(sum(durable_count),0) from agent_conversations",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap();
    let first_pages = cache_writes(&sql);
    drop(sql);
    let prefix_ms = before.elapsed().as_secs_f64() * 1000.;
    window.mark();
    if abrupt_exit && ledger.job.mode.logged() {
        assert!(
            prefix < ledger.facts.events,
            "recovery must exercise the acknowledged log suffix"
        );
    } else {
        assert_eq!(prefix, ledger.facts.events);
    }
    let before = Instant::now();
    let mut target = workload::open(path, ledger.job.mode, &observation);
    let pair_open_codec = observation.codec_delta(codec_before);
    let open_ms = before.elapsed().as_secs_f64() * 1000.;
    window.mark();
    let before = Instant::now();
    let facts = target.sql(|db| verify(db, &data));
    assert_eq!(facts, ledger.facts);
    verify_original(&mut target, &data, &ledger);
    let verify_ms = before.elapsed().as_secs_f64() * 1000.;
    window.mark();
    let before = Instant::now();
    let pages = target.finish();
    drop(target);
    let engine_io = observation.delta(io_before);
    let full_recovery_codec = observation.codec_delta(codec_before);
    let close_ms = before.elapsed().as_secs_f64() * 1000.;
    window.mark();
    Recovery {
        mode: ledger.job.mode,
        abrupt_exit,
        window: window.finish(),
        engine_io,
        pair_open_codec,
        full_recovery_codec,
        synthetic_ledger_and_expected_data_ms: preparation_ms,
        sql_events_at_first_open: prefix,
        sql_prefix_read_ms: prefix_ms,
        pair_open_and_replay_ms: open_ms,
        verification_and_original_replay_ms: verify_ms,
        sync_close_ms: close_ms,
        acknowledged_commands: ledger.acknowledgements.len(),
        facts,
        first_open_cache_pages_written: first_pages,
        paired_cache_pages_written: pages,
        power_loss_tested: false,
    }
}
