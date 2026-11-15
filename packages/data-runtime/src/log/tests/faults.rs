//! Faults wrap the published engine's default filesystem. No fake log engine,
//! checksum, recovery or synchronization implementation replaces production.
use super::*;
use raft_engine::env::{Handle, Permission, WriteExt};
use std::{
    io::{self, Seek, SeekFrom, Write},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

#[derive(Default)]
struct Faults {
    mode: AtomicUsize,
    syncs: AtomicUsize,
    writes: AtomicUsize,
    bytes: AtomicU64,
}
#[derive(Default)]
struct FileSystemProbe(Arc<Faults>);
struct ProbeHandle {
    inner: Arc<<DefaultFileSystem as FileSystem>::Handle>,
    state: Arc<Faults>,
}
struct ProbeWriter {
    inner: <DefaultFileSystem as FileSystem>::Writer,
    state: Arc<Faults>,
}

impl Handle for ProbeHandle {
    fn truncate(&self, offset: usize) -> io::Result<()> {
        self.inner.truncate(offset)
    }
    fn file_size(&self) -> io::Result<usize> {
        self.inner.file_size()
    }
    fn sync(&self) -> io::Result<()> {
        self.state.syncs.fetch_add(1, Ordering::SeqCst);
        match self.state.mode.load(Ordering::SeqCst) {
            3 => return Err(io::Error::other("synthetic sync error")),
            4 => panic!("synthetic sync panic"),
            _ => {}
        }
        self.inner.sync()
    }
}
impl Write for ProbeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.state.writes.fetch_add(1, Ordering::SeqCst);
        match self.state.mode.load(Ordering::SeqCst) {
            1 => return Err(io::Error::other("synthetic write error")),
            2 => panic!("synthetic write panic"),
            _ => {}
        }
        let count = self.inner.write(bytes)?;
        self.state.bytes.fetch_add(count as u64, Ordering::SeqCst);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for ProbeWriter {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.inner.seek(offset)
    }
}
impl WriteExt for ProbeWriter {
    fn truncate(&mut self, offset: usize) -> io::Result<()> {
        self.inner.truncate(offset)
    }
    fn allocate(&mut self, offset: usize, size: usize) -> io::Result<()> {
        self.inner.allocate(offset, size)
    }
}
impl FileSystem for FileSystemProbe {
    type Handle = ProbeHandle;
    type Reader = <DefaultFileSystem as FileSystem>::Reader;
    type Writer = ProbeWriter;
    fn create<P: AsRef<Path>>(&self, path: P) -> io::Result<Self::Handle> {
        Ok(ProbeHandle {
            inner: Arc::new(DefaultFileSystem.create(path)?),
            state: self.0.clone(),
        })
    }
    fn open<P: AsRef<Path>>(&self, path: P, perm: Permission) -> io::Result<Self::Handle> {
        Ok(ProbeHandle {
            inner: Arc::new(DefaultFileSystem.open(path, perm)?),
            state: self.0.clone(),
        })
    }
    fn delete<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        DefaultFileSystem.delete(path)
    }
    fn rename<P: AsRef<Path>>(&self, from: P, to: P) -> io::Result<()> {
        DefaultFileSystem.rename(from, to)
    }
    fn new_reader(&self, handle: Arc<Self::Handle>) -> io::Result<Self::Reader> {
        DefaultFileSystem.new_reader(handle.inner.clone())
    }
    fn new_writer(&self, handle: Arc<Self::Handle>) -> io::Result<Self::Writer> {
        Ok(ProbeWriter {
            inner: DefaultFileSystem.new_writer(handle.inner.clone())?,
            state: self.0.clone(),
        })
    }
}

#[test]
fn device_barrier_writes_a_nonempty_watermark_and_reaches_the_native_sync_handle() {
    let dir = tempfile::tempdir().unwrap();
    let probe = Arc::new(FileSystemProbe::default());
    let mut log =
        RaftLog::open_with_file_system(LogConfig::desktop(dir.path()), None, probe.clone())
            .unwrap();
    log.append(1, b"first", false).unwrap();
    log.append(2, b"second", false).unwrap();
    let syncs = probe.0.syncs.load(Ordering::SeqCst);
    let writes = probe.0.writes.load(Ordering::SeqCst);
    let bytes = probe.0.bytes.load(Ordering::SeqCst);
    let io = log.io_counters();
    let before = io.snapshot();
    assert_eq!(log.sync().unwrap(), 2);
    assert!(probe.0.syncs.load(Ordering::SeqCst) > syncs);
    assert!(probe.0.writes.load(Ordering::SeqCst) > writes);
    assert!(probe.0.bytes.load(Ordering::SeqCst) > bytes);
    let after = io.snapshot();
    assert!(after.sync_calls > before.sync_calls);
    assert!(after.write_calls > before.write_calls && after.write_bytes > before.write_bytes);
    assert_eq!(
        after.sync_calls,
        probe.0.syncs.load(Ordering::SeqCst) as u64
    );
    assert_eq!(
        after.write_calls,
        probe.0.writes.load(Ordering::SeqCst) as u64
    );
    assert_eq!(after.write_bytes, probe.0.bytes.load(Ordering::SeqCst));
    assert_eq!((after.write_errors, after.sync_errors), (0, 0));
    let synced = probe.0.syncs.load(Ordering::SeqCst);
    let written = probe.0.writes.load(Ordering::SeqCst);
    assert_eq!(log.sync().unwrap(), 2);
    assert_eq!(probe.0.syncs.load(Ordering::SeqCst), synced);
    assert_eq!(probe.0.writes.load(Ordering::SeqCst), written);
    assert_eq!(
        io.snapshot(),
        after,
        "an already-synced barrier does no delegate I/O"
    );
    assert_eq!(log.head().unwrap().accepted_position, 2);
    assert_eq!(log.read_range(1, 10, 100).unwrap().len(), 2);
    drop(log);
    let log = RaftLog::open(dir.path(), None).unwrap();
    assert_eq!(log.head().unwrap().synced_position, 2);
}

#[test]
fn capacity_preflight_rejects_before_any_engine_write_or_sync() {
    let dir = tempfile::tempdir().unwrap();
    let probe = Arc::new(FileSystemProbe::default());
    let mut config = LogConfig::desktop(dir.path());
    config.target_file_bytes = 64 * 1024;
    config.purge_threshold_bytes = 64 * 1024;
    config.rewrite_threshold_bytes = 64 * 1024;
    config.max_managed_file_bytes = 512 * 1024;
    config.maintenance_reserve_bytes = 128 * 1024;
    let mut log = RaftLog::open_with_file_system(config, None, probe.clone()).unwrap();
    let writes = probe.0.writes.load(Ordering::SeqCst);
    let syncs = probe.0.syncs.load(Ordering::SeqCst);
    assert!(matches!(
        log.append(1, &[1; 320 * 1024], true),
        Err(LogError::CapacityNotAccepted { .. })
    ));
    assert_eq!(probe.0.writes.load(Ordering::SeqCst), writes);
    assert_eq!(probe.0.syncs.load(Ordering::SeqCst), syncs);
    assert_eq!(log.head().unwrap().accepted_position, 0);
    log.append(1, b"smaller retry", false).unwrap();
}

#[test]
fn engine_write_errors_and_panics_poison_the_instance_until_reopen() {
    for mode in [1, 2, 3, 4] {
        let dir = tempfile::tempdir().unwrap();
        let probe = Arc::new(FileSystemProbe::default());
        let mut log =
            RaftLog::open_with_file_system(LogConfig::desktop(dir.path()), None, probe.clone())
                .unwrap();
        log.append(1, b"confirmed first", false).unwrap();
        let io = log.io_counters();
        let before = io.snapshot();
        probe.0.mode.store(mode, Ordering::SeqCst);
        let error = log.append(2, b"uncertain second", mode >= 3).unwrap_err();
        assert!(matches!(
            error,
            LogError::Engine { .. } | LogError::EnginePanic { .. }
        ));
        assert!(matches!(log.head(), Err(LogError::Poisoned)));
        assert!(matches!(
            log.append(3, b"must not write", false),
            Err(LogError::Poisoned)
        ));
        assert!(matches!(log.read_range(1, 1, 100), Err(LogError::Poisoned)));
        assert!(matches!(log.sync(), Err(LogError::Poisoned)));
        assert!(matches!(
            log.compact_prefix(1, false),
            Err(LogError::Poisoned)
        ));
        assert!(matches!(log.purge(), Err(LogError::Poisoned)));
        let after = io.snapshot();
        if mode <= 2 {
            assert!(after.write_calls > before.write_calls);
            assert_eq!(after.write_bytes, before.write_bytes);
            assert_eq!(
                after.write_errors - before.write_errors,
                u64::from(mode == 1)
            );
        } else {
            assert!(after.sync_calls > before.sync_calls);
            assert_eq!(after.sync_errors - before.sync_errors, u64::from(mode == 3));
        }
        probe.0.mode.store(0, Ordering::SeqCst);
        drop(log);
        // A sync panic may have appended the second batch. Reopen reconciles
        // actual state; the failed owner never asserted that it was unexecuted.
        let mut log = RaftLog::open(dir.path(), None).unwrap();
        let records = log.read_range(1, 2, 100).unwrap();
        assert_eq!(records[0].payload, b"confirmed first");
        let next = log.head().unwrap().accepted_position + 1;
        log.append(next, b"after reconciliation", false).unwrap();
    }
}

#[test]
fn failed_compaction_is_not_reported_as_a_successful_prefix_reclaim() {
    let dir = tempfile::tempdir().unwrap();
    let probe = Arc::new(FileSystemProbe::default());
    let mut log =
        RaftLog::open_with_file_system(LogConfig::desktop(dir.path()), None, probe.clone())
            .unwrap();
    log.append(1, b"retain", false).unwrap();
    probe.0.mode.store(1, Ordering::SeqCst);
    assert!(log.compact_prefix(1, false).is_err());
    assert!(matches!(log.head(), Err(LogError::Poisoned)));
    probe.0.mode.store(0, Ordering::SeqCst);
    drop(log);
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    assert_eq!(log.head().unwrap().compacted_position, 0);
    assert_eq!(log.read_range(1, 1, 100).unwrap()[0].payload, b"retain");
}
