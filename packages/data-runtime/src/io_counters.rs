//! Delegated Raft file I/O, independent of logical extent and OS/device writes.
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Cumulative attempts at the wrapped `Write::write` and `Handle::sync` ports.
/// `write_bytes` sums only successful returned byte counts. An error may have
/// written an unknown partial range; neither requested length nor extent is a
/// substitute. Panics count as attempts but are not returned I/O errors.
///
/// This excludes resize/allocation, flush, metadata/directory operations and
/// engine control files that bypass its FileSystem port. It is not physical
/// disk traffic, allocated blocks, SQLite I/O or a durability attestation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogIoSnapshot {
    pub write_calls: u64,
    pub write_bytes: u64,
    pub write_errors: u64,
    pub sync_calls: u64,
    pub sync_errors: u64,
}

#[derive(Debug, Default)]
struct Counters {
    write_calls: AtomicU64,
    write_bytes: AtomicU64,
    write_errors: AtomicU64,
    sync_calls: AtomicU64,
    sync_errors: AtomicU64,
}

/// A lightweight observer retained independently of an owned log's lifetime.
/// Create a fresh handle per log opening and clone it for sampling, including
/// before open and after close. There is no global registration or counter reset.
/// Relaxed atomics add no filesystem I/O or change in durability ordering;
/// concurrent snapshots can span operations, so take window boundaries while
/// the owner is quiescent. The counters do not own or keep its files open.
#[derive(Clone, Debug, Default)]
pub struct LogIoCounters(Arc<Counters>);

impl LogIoCounters {
    pub fn snapshot(&self) -> LogIoSnapshot {
        LogIoSnapshot {
            write_calls: self.0.write_calls.load(Ordering::Relaxed),
            write_bytes: self.0.write_bytes.load(Ordering::Relaxed),
            write_errors: self.0.write_errors.load(Ordering::Relaxed),
            sync_calls: self.0.sync_calls.load(Ordering::Relaxed),
            sync_errors: self.0.sync_errors.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn write(&self, action: impl FnOnce() -> io::Result<usize>) -> io::Result<usize> {
        self.0.write_calls.fetch_add(1, Ordering::Relaxed);
        let result = action();
        match &result {
            Ok(count) => {
                self.0
                    .write_bytes
                    .fetch_add(*count as u64, Ordering::Relaxed);
            }
            Err(_) => {
                self.0.write_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
        result
    }

    pub(crate) fn sync(&self, action: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        self.0.sync_calls.fetch_add(1, Ordering::Relaxed);
        let result = action();
        if result.is_err() {
            self.0.sync_errors.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
}
