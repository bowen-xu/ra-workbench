//! Actual journal writers and actual coordinator observations; no simulated batching.
use super::runtime_metrics::{EngineIo, Observation};
#[path = "workload/storage_tests.rs"]
mod storage_tests;
use super::{metrics::*, scenario::*, verification::*, *};
use research_data_runtime::{
    CodecSnapshot, CoordinatorConfig, Durability, LogConfig, WriteMode, WriteOptions,
};
use rusqlite::Connection;
use std::{path::Path, sync::Arc, time::Instant};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Measurement {
    pub job: Job,
    pub initialization: WindowMeasurement,
    pub complete_write_window: WindowMeasurement,
    pub initialization_engine_io: Option<EngineIo>,
    pub append_loop_engine_io: Option<EngineIo>,
    pub complete_write_engine_io: Option<EngineIo>,
    pub initialization_codec: Option<CodecSnapshot>,
    pub append_loop_codec: Option<CodecSnapshot>,
    pub complete_write_codec: Option<CodecSnapshot>,
    pub measured_event_count: usize,
    pub canonical_business_text_bytes: usize,
    pub actual_text_delta_count: usize,
    pub actual_text_delta_bytes: usize,
    pub append_p50_us: f64,
    pub append_p95_us: f64,
    pub query_us: Vec<f64>,
    pub loop_ms: f64,
    pub final_verification_ms: f64,
    pub final_sync_close_ms: f64,
    pub sql: SqlCounts,
    pub sqlite_cache_pages_written: u64,
    pub file_ready: FileOccupancy,
    pub file_sampled_peak: FileOccupancy,
    pub file_end: FileOccupancy,
    pub pending_commands_sampled_peak_after_ack: u64,
    pub pending_bytes_sampled_peak_after_ack: Option<u64>,
    pub configured: serde_json::Value,
    pub actual_pragma: serde_json::Value,
    pub facts: Facts,
    pub acknowledged_commands: usize,
    pub first_text_ack_us: Option<f64>,
    pub first_query_text_observed_us: Option<f64>,
    pub subscription_display_latency_us: Option<f64>,
    pub engine_write_calls: Option<u64>,
    pub engine_sync_calls: Option<u64>,
    pub engine_write_bytes: Option<u64>,
    pub engine_write_errors: Option<u64>,
    pub engine_sync_errors: Option<u64>,
    pub serialized_command_bytes: Option<u64>,
    pub serialization_cpu_us: Option<u64>,
    pub clean_recovery: Option<super::recovery::Recovery>,
}
pub(super) enum Target {
    Legacy(Connection),
    Automatic(SharedDatabase),
}
impl Target {
    pub fn sql<T>(&self, operation: impl FnOnce(&Connection) -> T) -> T {
        match self {
            Self::Legacy(db) => operation(db),
            Self::Automatic(db) => operation(&db.lock().unwrap()),
        }
    }
    pub fn append(&mut self, m: &Mutation, mode: Mode) -> Ack {
        match self {
            Self::Legacy(db) => {
                let result = crate::agent::repository::append_batch(
                    db,
                    m.conversation,
                    m.batch,
                    m.expected,
                    std::slice::from_ref(&m.event),
                )
                .unwrap();
                Ack {
                    conversation: m.conversation.as_uuid().to_string(),
                    batch: m.batch,
                    operation: None,
                    position: None,
                    result,
                    replayed: None,
                }
            }
            Self::Automatic(db) => {
                let receipt = db.write(m.command(), mode.options()).unwrap();
                Ack {
                    conversation: m.conversation.as_uuid().to_string(),
                    batch: m.batch,
                    operation: Some(receipt.operation_id),
                    position: Some(receipt.position),
                    result: receipt.result,
                    replayed: Some(receipt.replayed),
                }
            }
        }
    }
    fn pending(&self) -> (u64, Option<u64>) {
        match self {
            Self::Legacy(_) => (0, None),
            Self::Automatic(db) => {
                let state = db.benchmark_snapshot().unwrap();
                (
                    state.accepted_position - state.materialized_position,
                    Some(u64::try_from(state.pending_bytes).unwrap()),
                )
            }
        }
    }
    pub fn finish(&self) -> u64 {
        let pages = self.sql(|db| {
            db.execute_batch("pragma synchronous=full;pragma wal_checkpoint(truncate);")
                .unwrap();
            cache_writes(db)
        });
        if let Self::Automatic(db) = self {
            db.shutdown().unwrap();
        }
        pages
    }
}
pub(super) fn connection(path: &Path, mode: Mode) -> Connection {
    let sql = Connection::open(path.join("fixture.sqlite3")).unwrap();
    sql.execute_batch("pragma foreign_keys=on;").unwrap();
    super::super::operation_identity::register(&sql).unwrap();
    sql.execute_batch(if matches!(mode, Mode::LegacyNormal) {
        "pragma journal_mode=wal;pragma synchronous=normal;"
    } else {
        "pragma journal_mode=wal;pragma synchronous=full;"
    })
    .unwrap();
    sql
}
pub(super) fn open(path: &Path, mode: Mode, observation: &Observation) -> Target {
    let sql = connection(path, mode);
    if mode.logged() {
        Target::Automatic(
            Database::benchmark_open_observed(
                sql,
                &path.join("data-log"),
                observation
                    .counters()
                    .expect("log observation starts before open"),
                observation
                    .codec_counters()
                    .expect("codec observation starts before open"),
            )
            .unwrap(),
        )
    } else {
        Target::Legacy(sql)
    }
}
fn fixture(
    path: &Path,
    job: &Job,
    data: &Dataset,
    counts: &Arc<SqlCounters>,
    observation: &Observation,
) -> (Target, Vec<Ack>) {
    std::fs::create_dir_all(path).unwrap();
    let mut sql = connection(path, job.mode);
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch(if job.mode == Mode::LegacyNormal {
        "pragma synchronous=normal;"
    } else {
        "pragma synchronous=full;"
    })
    .unwrap();
    SqlCounters::install(&sql, counts);
    let mut target = if job.mode.logged() {
        Target::Automatic(
            Database::benchmark_open_observed(
                sql,
                &path.join("data-log"),
                observation
                    .counters()
                    .expect("log observation starts before open"),
                observation
                    .codec_counters()
                    .expect("codec observation starts before open"),
            )
            .unwrap(),
        )
    } else {
        Target::Legacy(sql)
    };
    for c in &data.conversations {
        match &mut target {
            Target::Legacy(db) => {
                crate::agent::repository::create_conversation(db, *c, "Synthetic benchmark")
                    .unwrap()
            }
            Target::Automatic(db) => {
                db.write(
                    journal::JournalCommand::new(
                        *c,
                        Uuid::new_v5(c.as_uuid(), b"create"),
                        0,
                        vec![],
                        Some("Synthetic benchmark".into()),
                        1_790_000_000_000,
                    )
                    .unwrap(),
                    WriteOptions {
                        mode: WriteMode::Immediate,
                        durability: Durability::Device,
                    },
                )
                .unwrap();
            }
        }
    }
    let acknowledgements = data
        .setup
        .iter()
        .map(|m| target.append(m, job.mode))
        .collect();
    target.sql(|db| {
        db.execute_batch("pragma wal_checkpoint(truncate);")
            .unwrap()
    });
    counts.reset();
    (target, acknowledgements)
}
fn configuration(path: &Path) -> serde_json::Value {
    let c = CoordinatorConfig::default();
    let l = LogConfig::desktop(path.join("data-log"));
    serde_json::json!({"materializeRecords":c.materialize_records,"materializeBytes":c.materialize_bytes,"maxPendingRecords":c.max_pending_records,"maxPendingBytes":c.max_pending_bytes,"maxCommandBytes":c.max_command_bytes,"replayBatchRecords":c.replay_batch_records,"reclaimAfterRecords":c.reclaim_after_records,"targetFileBytes":l.target_file_bytes,"purgeThresholdBytes":l.purge_threshold_bytes,"rewriteThresholdBytes":l.rewrite_threshold_bytes,"maxManagedFileBytes":l.max_managed_file_bytes,"maintenanceReserveBytes":l.maintenance_reserve_bytes,"logRecoveryThreads":1,"logRecycle":false})
}
fn pragma(target: &Target) -> serde_json::Value {
    target.sql(|db|serde_json::json!({"journalMode":db.query_row("pragma journal_mode",[],|row|row.get::<_,String>(0)).unwrap(),"synchronous":db.query_row("pragma synchronous",[],|row|row.get::<_,u64>(0)).unwrap(),"foreignKeys":db.query_row("pragma foreign_keys",[],|row|row.get::<_,u64>(0)).unwrap(),"pageSize":db.query_row("pragma page_size",[],|row|row.get::<_,u64>(0)).unwrap(),"walAutocheckpoint":db.query_row("pragma wal_autocheckpoint",[],|row|row.get::<_,u64>(0)).unwrap()}))
}
pub(super) fn run(path: &Path, job: Job, crash: bool) -> Option<Measurement> {
    let initialization = Window::start();
    let observation = Observation::new(job.mode);
    let init_io_before = observation.snapshot();
    let init_codec_before = observation.codec_snapshot();
    assert_fixture(path);
    let data = dataset(&job);
    let expected_facts = expected(&data);
    let counts = Arc::new(SqlCounters::default());
    let (mut target, setup_acknowledgements) = fixture(path, &job, &data, &counts, &observation);
    let actual_pragma = pragma(&target);
    assert_eq!(
        actual_pragma["synchronous"],
        if job.mode == Mode::LegacyNormal { 1 } else { 2 }
    );
    assert_eq!(actual_pragma["journalMode"], "wal");
    assert_eq!(actual_pragma["foreignKeys"], 1);
    let configured = configuration(path);
    let initialization = initialization.finish();
    let initialization_engine_io = observation.delta(init_io_before);
    let initialization_codec = observation.codec_delta(init_codec_before);
    let cache_start = target.sql(cache_writes);
    let file_ready = FileOccupancy::sample(path);
    let mut peak = file_ready;
    let mut prefix = Prefix::default();
    for m in &data.setup {
        prefix.accept(m);
    }
    let mut acknowledged = vec![];
    let mut latency = vec![];
    let mut query_us = vec![];
    let mut pending_peak = 0;
    let mut pending_bytes_peak = job.mode.logged().then_some(0_u64);
    let mut first_text_ack = None;
    let mut first_query_text = None;
    let mut has_text = false;
    let mut window = Window::start();
    let write_io_before = observation.snapshot();
    let write_codec_before = observation.codec_snapshot();
    let started = Instant::now();
    for (index, m) in data.measured.iter().enumerate() {
        let before = Instant::now();
        acknowledged.push(target.append(m, job.mode));
        latency.push(before.elapsed().as_nanos() as u64);
        if matches!(
            m.event.payload(),
            research_agent_core::protocol::EventPayload::TextDelta { .. }
        ) {
            has_text = true;
            if first_text_ack.is_none() {
                first_text_ack = Some(started.elapsed().as_secs_f64() * 1e6);
            }
        }
        prefix.accept(m);
        let (pending, bytes) = target.pending();
        pending_peak = pending_peak.max(pending);
        pending_bytes_peak = pending_bytes_peak.zip(bytes).map(|(old, new)| old.max(new));
        if !crash
            && job
                .query_every
                .is_some_and(|every| (index + 1) % every == 0)
        {
            let query = Instant::now();
            target.sql(|db| prefix.verify(db));
            query_us.push(query.elapsed().as_secs_f64() * 1e6);
            if has_text && first_query_text.is_none() {
                first_query_text = Some(started.elapsed().as_secs_f64() * 1e6);
            }
        }
        if index % 32 == 0 {
            window.mark();
            peak = peak.max(FileOccupancy::sample(path));
        }
    }
    let loop_ms = started.elapsed().as_secs_f64() * 1000.;
    let append_loop_engine_io = observation.delta(write_io_before);
    let append_loop_codec = observation.codec_delta(write_codec_before);
    let ledger = Ledger {
        job: job.clone(),
        setup_acknowledgements,
        acknowledgements: acknowledged,
        facts: expected_facts,
    };
    if crash {
        if job.mode.logged() {
            assert!(
                target.pending().0 > 0,
                "crash fixture must retain an actual unmaterialized suffix"
            );
        }
        std::fs::write(
            path.join("acknowledged.json"),
            serde_json::to_vec(&ledger).unwrap(),
        )
        .unwrap();
        // All original commands are acknowledged; bypass every Drop/shutdown.
        std::process::exit(0);
    }
    let before = Instant::now();
    let facts = target.sql(|db| verify(db, &data));
    assert_eq!(facts, ledger.facts);
    let final_verification_ms = before.elapsed().as_secs_f64() * 1000.;
    window.mark();
    peak = peak.max(FileOccupancy::sample(path));
    let before = Instant::now();
    let cache_end = target.finish();
    let sql = counts.snapshot();
    drop(target);
    let complete_write_engine_io = observation.delta(write_io_before);
    let complete_write_codec = observation.codec_delta(write_codec_before);
    let final_sync_close_ms = before.elapsed().as_secs_f64() * 1000.;
    window.mark();
    let file_end = FileOccupancy::sample(path);
    peak = peak.max(file_end);
    let complete_write_window = window.finish();
    std::fs::write(
        path.join("acknowledged.json"),
        serde_json::to_vec(&ledger).unwrap(),
    )
    .unwrap();
    let business = data
        .measured
        .iter()
        .filter_map(|m| match m.event.payload() {
            research_agent_core::protocol::EventPayload::ItemAccepted { text, .. } => {
                Some(text.len())
            }
            _ => None,
        })
        .sum();
    let deltas = data
        .measured
        .iter()
        .filter_map(|m| match m.event.payload() {
            research_agent_core::protocol::EventPayload::TextDelta { text, .. } => Some(text.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    Some(Measurement {
        job,
        initialization,
        complete_write_window,
        initialization_engine_io,
        append_loop_engine_io,
        complete_write_engine_io,
        initialization_codec,
        append_loop_codec,
        complete_write_codec,
        measured_event_count: data.measured.len(),
        canonical_business_text_bytes: business,
        actual_text_delta_count: deltas.len(),
        actual_text_delta_bytes: deltas.iter().sum(),
        append_p50_us: percentile(&latency, 50) as f64 / 1000.,
        append_p95_us: percentile(&latency, 95) as f64 / 1000.,
        query_us,
        loop_ms,
        final_verification_ms,
        final_sync_close_ms,
        sql,
        sqlite_cache_pages_written: cache_end - cache_start,
        file_ready,
        file_sampled_peak: peak,
        file_end,
        pending_commands_sampled_peak_after_ack: pending_peak,
        pending_bytes_sampled_peak_after_ack: pending_bytes_peak,
        configured,
        actual_pragma,
        facts,
        acknowledged_commands: ledger.acknowledgements.len(),
        first_text_ack_us: first_text_ack,
        first_query_text_observed_us: first_query_text,
        subscription_display_latency_us: None,
        engine_write_calls: complete_write_engine_io.map(|io| io.write_attempts),
        engine_sync_calls: complete_write_engine_io.map(|io| io.sync_attempts),
        engine_write_bytes: complete_write_engine_io.map(|io| io.reported_write_bytes),
        engine_write_errors: complete_write_engine_io.map(|io| io.write_errors),
        engine_sync_errors: complete_write_engine_io.map(|io| io.sync_errors),
        serialized_command_bytes: complete_write_codec.map(|value| value.command_encoded_bytes),
        serialization_cpu_us: None,
        clean_recovery: None,
    })
}

#[test]
fn observed_pair_reports_actual_pending_bytes_then_zero_after_projection_and_keeps_close_counts() {
    let root = tempfile::tempdir().unwrap();
    let job = Job {
        mode: Mode::AutoProcess,
        shape: Shape::Short,
        query_every: None,
        fragments: 128,
        fragment_bytes: 128,
    };
    let data = dataset(&job);
    let counts = Arc::new(SqlCounters::default());
    let observation = Observation::new(job.mode);
    let before_open = observation.snapshot();
    let (mut target, _) = fixture(root.path(), &job, &data, &counts, &observation);
    let initialized = observation.delta(before_open).unwrap();
    assert!(initialized.write_attempts > 0);
    assert!(initialized.reported_write_bytes > 0);
    assert_eq!(target.pending(), (0, Some(0)));
    let ready = observation.snapshot();
    for mutation in data.measured.iter().take(3) {
        target.append(mutation, job.mode);
    }
    let (commands, bytes) = target.pending();
    assert_eq!(commands, 3);
    assert!(bytes.unwrap() > 0);
    target.sql(|_| ());
    assert_eq!(target.pending(), (0, Some(0)));
    let before_close = observation.snapshot().unwrap();
    target.finish();
    drop(target);
    let after_close = observation.snapshot().unwrap();
    assert!(after_close.write_attempts >= before_close.write_attempts);
    assert!(after_close.sync_attempts >= before_close.sync_attempts);
    let complete = observation.delta(ready).unwrap();
    assert!(complete.write_attempts > 0);
    assert!(complete.reported_write_bytes > 0);
    assert!(complete.sync_attempts > 0);
    assert_eq!(complete.write_errors, 0);
    assert_eq!(complete.sync_errors, 0);
}
