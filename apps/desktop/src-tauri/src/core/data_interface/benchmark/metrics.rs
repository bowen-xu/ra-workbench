//! In-process counters over synthetic data only. File sizes are occupancy,
//! never labeled as total disk writes; OS counters cover the complete process.
use rusqlite::{Connection, ffi, functions::FunctionFlags};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CStr, c_void},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Default)]
pub(super) struct SqlCounters {
    commits: AtomicU64,
    body_updates: AtomicU64,
    rewritten_body_bytes: AtomicU64,
    materialization_batches: AtomicU64,
    materialized_records: AtomicU64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) struct SqlCounts {
    pub commits: u64,
    pub body_updates: u64,
    pub rewritten_body_bytes: u64,
    pub materialization_batches: u64,
    pub materialized_records: u64,
}
impl SqlCounters {
    pub(super) fn install(db: &Connection, counts: &Arc<Self>) {
        let body = Arc::clone(counts);
        db.create_scalar_function(
            "synthetic_body_write",
            1,
            FunctionFlags::SQLITE_UTF8,
            move |context| {
                let bytes = context
                    .get_raw(0)
                    .as_bytes()
                    .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                body.body_updates.fetch_add(1, Ordering::Relaxed);
                body.rewritten_body_bytes
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch("create temp trigger synthetic_body_counter after update of body on main.agent_messages begin select synthetic_body_write(new.body); end;").unwrap();
        let projection = counts.clone();
        db.create_scalar_function(
            "synthetic_materialized",
            1,
            FunctionFlags::SQLITE_UTF8,
            move |context| {
                projection
                    .materialization_batches
                    .fetch_add(1, Ordering::Relaxed);
                projection
                    .materialized_records
                    .fetch_add(context.get::<u64>(0)?, Ordering::Relaxed);
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch("create temp trigger synthetic_materialized_counter after update of materialized_position on main.data_runtime_state when new.materialized_position>old.materialized_position begin select synthetic_materialized(new.materialized_position-old.materialized_position); end;").unwrap();
        // SAFETY: sqlite invokes this callback synchronously; `counts` stays alive
        // until after the connection and its data owner have been dropped.
        let status = unsafe {
            ffi::sqlite3_trace_v2(
                db.handle(),
                ffi::SQLITE_TRACE_STMT,
                Some(trace_commit),
                Arc::as_ptr(counts).cast_mut().cast::<c_void>(),
            )
        };
        assert_eq!(status, ffi::SQLITE_OK);
    }
    pub(super) fn reset(&self) {
        self.commits.store(0, Ordering::Relaxed);
        self.body_updates.store(0, Ordering::Relaxed);
        self.rewritten_body_bytes.store(0, Ordering::Relaxed);
        self.materialization_batches.store(0, Ordering::Relaxed);
        self.materialized_records.store(0, Ordering::Relaxed);
    }
    pub(super) fn snapshot(&self) -> SqlCounts {
        SqlCounts {
            commits: self.commits.load(Ordering::Relaxed),
            body_updates: self.body_updates.load(Ordering::Relaxed),
            rewritten_body_bytes: self.rewritten_body_bytes.load(Ordering::Relaxed),
            materialization_batches: self.materialization_batches.load(Ordering::Relaxed),
            materialized_records: self.materialized_records.load(Ordering::Relaxed),
        }
    }
}
unsafe extern "C" fn trace_commit(
    _event: u32,
    context: *mut c_void,
    statement: *mut c_void,
    _sql: *mut c_void,
) -> i32 {
    // SAFETY: the registered STMT trace receives the live sqlite3_stmt and the
    // stable Arc allocation registered above, for the duration of this callback.
    let (counter, sql) = unsafe {
        (
            &*context.cast::<SqlCounters>(),
            ffi::sqlite3_sql(statement.cast()),
        )
    };
    if !sql.is_null() {
        // SAFETY: sqlite3_sql returns NUL-terminated SQL owned by the live stmt.
        let bytes = unsafe { CStr::from_ptr(sql) }.to_bytes();
        let command = bytes
            .split(|byte| byte.is_ascii_whitespace())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        if command.eq_ignore_ascii_case(b"COMMIT") || command.eq_ignore_ascii_case(b"END") {
            counter.commits.fetch_add(1, Ordering::Relaxed);
        }
    }
    0
}
pub(super) fn cache_writes(db: &Connection) -> u64 {
    let mut current = 0;
    let mut high = 0;
    // SAFETY: sqlite handle is exclusively borrowed and both output ints are live.
    let code = unsafe {
        ffi::sqlite3_db_status(
            db.handle(),
            ffi::SQLITE_DBSTATUS_CACHE_WRITE,
            &mut current,
            &mut high,
            0,
        )
    };
    assert_eq!(code, ffi::SQLITE_OK);
    current as u64
}
#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize)]
pub(crate) struct ProcessSample {
    pub disk_written_bytes: Option<u64>,
    pub disk_read_bytes: Option<u64>,
    pub user_cpu_us: Option<u64>,
    pub system_cpu_us: Option<u64>,
    pub resident_bytes: Option<u64>,
}
#[cfg(target_os = "macos")]
pub(crate) fn process_sample() -> ProcessSample {
    use nix::libc;
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::uninit();
    // SAFETY: the flavor and allocated output struct match Apple's public ABI;
    // only the calling process is inspected, never another application.
    let code = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    if code != 0 {
        return ProcessSample::default();
    }
    // SAFETY: successful proc_pid_rusage initialized the complete V4 output.
    let usage = unsafe { usage.assume_init() };
    let (user_cpu_us, system_cpu_us) = cpu_sample();
    ProcessSample {
        disk_written_bytes: Some(usage.ri_diskio_byteswritten),
        disk_read_bytes: Some(usage.ri_diskio_bytesread),
        user_cpu_us,
        system_cpu_us,
        resident_bytes: Some(usage.ri_resident_size),
    }
}
#[cfg(target_os = "macos")]
fn cpu_sample() -> (Option<u64>, Option<u64>) {
    use nix::libc;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: RUSAGE_SELF uses the exact allocated rusage ABI. No other process
    // is inspected; timeval explicitly supplies seconds and microseconds.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return (None, None);
    }
    // SAFETY: successful getrusage initialized the complete output.
    let usage = unsafe { usage.assume_init() };
    let micros = |time: libc::timeval| {
        u64::try_from(time.tv_sec)
            .ok()
            .and_then(|sec| sec.checked_mul(1_000_000))
            .zip(u64::try_from(time.tv_usec).ok())
            .and_then(|(sec, micro)| sec.checked_add(micro))
    };
    (micros(usage.ru_utime), micros(usage.ru_stime))
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct WindowMeasurement {
    pub wall_ms: f64,
    pub user_cpu_us: Option<u64>,
    pub system_cpu_us: Option<u64>,
    pub disk_read_bytes: Option<u64>,
    pub disk_written_bytes: Option<u64>,
    pub resident_before_bytes: Option<u64>,
    pub resident_after_bytes: Option<u64>,
    pub resident_sampled_peak_bytes: Option<u64>,
}
pub(super) struct Window {
    started: std::time::Instant,
    before: ProcessSample,
    peak: Option<u64>,
}
impl Window {
    pub fn start() -> Self {
        let started = std::time::Instant::now();
        let before = process_sample();
        Self {
            started,
            before,
            peak: before.resident_bytes,
        }
    }
    pub fn mark(&mut self) {
        let now = process_sample();
        self.peak = self.peak.zip(now.resident_bytes).map(|(a, b)| a.max(b));
    }
    pub fn finish(mut self) -> WindowMeasurement {
        let end = process_sample();
        self.peak = self.peak.zip(end.resident_bytes).map(|(a, b)| a.max(b));
        let delta = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| b.saturating_sub(a));
        WindowMeasurement {
            wall_ms: self.started.elapsed().as_secs_f64() * 1000.,
            user_cpu_us: delta(self.before.user_cpu_us, end.user_cpu_us),
            system_cpu_us: delta(self.before.system_cpu_us, end.system_cpu_us),
            disk_read_bytes: delta(self.before.disk_read_bytes, end.disk_read_bytes),
            disk_written_bytes: delta(self.before.disk_written_bytes, end.disk_written_bytes),
            resident_before_bytes: self.before.resident_bytes,
            resident_after_bytes: end.resident_bytes,
            resident_sampled_peak_bytes: self.peak,
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn process_sample() -> ProcessSample {
    ProcessSample::default()
}
#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize)]
pub(super) struct FileOccupancy {
    pub sql_bytes: u64,
    pub wal_bytes: u64,
    pub log_bytes: u64,
}
impl FileOccupancy {
    pub(super) fn sample(path: &Path) -> Self {
        Self {
            sql_bytes: length(&path.join("fixture.sqlite3")),
            wal_bytes: length(&path.join("fixture.sqlite3-wal")),
            log_bytes: directory_bytes(&path.join("data-log")),
        }
    }
    pub(super) fn max(self, other: Self) -> Self {
        Self {
            sql_bytes: self.sql_bytes.max(other.sql_bytes),
            wal_bytes: self.wal_bytes.max(other.wal_bytes),
            log_bytes: self.log_bytes.max(other.log_bytes),
        }
    }
}
fn length(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |metadata| metadata.len())
}
fn directory_bytes(path: &Path) -> u64 {
    std::fs::read_dir(path).map_or(0, |entries| {
        entries
            .map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    directory_bytes(&path)
                } else {
                    length(&path)
                }
            })
            .sum()
    })
}
pub(super) fn percentile(values: &[u64], percent: usize) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}
