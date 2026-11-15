//! Ordered log adapter over published raft-engine 0.4.2. This owner never
//! projects SQL, interprets mutation bytes or decides which prefix is reclaimable.
use std::{
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::Arc,
};

use raft_engine::{
    Command, Engine, LogBatch,
    env::{DefaultFileSystem, FileSystem},
};
use uuid::Uuid;

use crate::{
    LogConfig, LogError, LogIoCounters, LogResult,
    envelope::{EnvelopeIndex, LOG_SCHEMA_VERSION, LogEnvelope, LogState},
    extent::{Account, ExtentFileSystem, ExtentUsage, QuotaFailure, ReservationKind},
};

mod opening;

const GROUP: u64 = 1;
const STATE_KEY: &[u8] = b"research.data_runtime.state.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogHead {
    pub epoch: Uuid,
    pub accepted_position: u64,
    pub synced_position: u64,
    pub compacted_position: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRecord {
    pub position: u64,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Maintenance {
    pub compaction_suggested: bool,
    /// Sum of the greater of logical size and allocated blocks for each file.
    /// This is not the application's total disk usage or a retention guarantee.
    pub bytes_on_disk: u64,
    pub logical_bytes: u64,
    pub files: usize,
}

/// Single sequential owner. Generic filesystem injection exercises the real
/// engine's I/O and panic paths in tests; production uses its default filesystem.
pub struct RaftLog<F: FileSystem = DefaultFileSystem> {
    engine: Engine<ExtentFileSystem<F>>,
    account: Arc<Account>,
    io: LogIoCounters,
    target_file_bytes: u64,
    maintenance_trigger_bytes: u64,
    path: PathBuf,
    head: LogHead,
    poisoned: bool,
}

impl<F: FileSystem> RaftLog<F> {
    /// Clone the per-opening observer without touching or retaining log files.
    pub fn io_counters(&self) -> LogIoCounters {
        self.io.clone()
    }
    pub fn head(&self) -> LogResult<LogHead> {
        self.ensure_active()?;
        Ok(self.head)
    }

    /// Actual directory held by this engine, including for paired backups.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn extent_usage(&self) -> LogResult<ExtentUsage> {
        self.account.usage().map_err(|source| LogError::Io {
            operation: "read log extent quota",
            source,
        })
    }
    pub(crate) fn maintenance_hint(&self) -> LogResult<(bool, u64)> {
        self.ensure_active()?;
        let usage = self.extent_usage()?;
        let engine_slots = self.engine.get_used_size() as u64;
        Ok((
            usage.managed_file_extent_bytes >= self.maintenance_trigger_bytes
                || engine_slots
                    >= self
                        .maintenance_trigger_bytes
                        .saturating_add(self.target_file_bytes * 2),
            usage.managed_file_extent_bytes,
        ))
    }
    pub(crate) fn maintenance_step_bytes(&self) -> u64 {
        self.target_file_bytes
    }

    /// A successful unsynced append has reached the engine's file write path;
    /// it does not claim stable-device durability. Positions never overwrite.
    pub fn append(&mut self, position: u64, payload: &[u8], sync: bool) -> LogResult<()> {
        self.ensure_active()?;
        if self.head.accepted_position.checked_add(1) != Some(position) || position == u64::MAX {
            return Err(LogError::InvalidArgument(
                "append position must be the next global position",
            ));
        }
        let mut entry = LogEnvelope::new();
        entry.schema_version = LOG_SCHEMA_VERSION;
        entry.store_epoch = self.head.epoch.as_bytes().to_vec();
        entry.position = position;
        entry.payload = payload.to_vec();
        let mut next = self.head;
        next.accepted_position = position;
        if sync {
            next.synced_position = position;
        }
        let mut batch = LogBatch::default();
        batch
            .add_entries::<EnvelopeIndex>(GROUP, &[entry])
            .map_err(|source| LogError::Encoding {
                operation: "encode append",
                source,
            })?;
        put_state(&mut batch, next)?;
        let reservation = batch.approximate_size() as u64 + self.target_file_bytes + 64 * 1024;
        let _reservation = self
            .account
            .reserve(reservation, ReservationKind::Admission)
            .map_err(capacity_error)?;
        self.call("append", |engine| engine.write(&mut batch, sync))?;
        self.head = next;
        Ok(())
    }

    /// Bounded by record count and opaque payload bytes. An oversized first
    /// record is an explicit error, never an empty page that stalls a projector.
    pub fn read_range(
        &mut self,
        start: u64,
        max_records: usize,
        max_bytes: usize,
    ) -> LogResult<Vec<LogRecord>> {
        self.ensure_active()?;
        if start == 0 || max_records == 0 || max_bytes == 0 {
            return Err(LogError::InvalidArgument("read bounds must be nonzero"));
        }
        if start <= self.head.compacted_position {
            return Err(LogError::Compacted);
        }
        if start > self.head.accepted_position {
            return Ok(vec![]);
        }
        let count = u64::try_from(max_records).unwrap_or(u64::MAX);
        let end = start
            .saturating_add(count)
            .min(self.head.accepted_position + 1);
        // 0.4.2 engine.rs:576–600 caches by FileBlockHandle in thread-local
        // storage without an engine identity. Both get_entry and batch fetch
        // use it. A fresh scoped reader prevents cross-directory/reopen aliasing
        // without a private API, fork, permanent worker or second log format.
        let head = self.head;
        self.isolated("read entry", move |engine| {
            read_page(engine, head, start, end, max_bytes)
        })
    }

    fn isolated<T: Send>(
        &mut self,
        operation: &'static str,
        action: impl FnOnce(&Engine<ExtentFileSystem<F>>) -> LogResult<T> + Send,
    ) -> LogResult<T> {
        self.ensure_active()?;
        let engine = &self.engine;
        let result = std::thread::scope(|scope| {
            let reader = std::thread::Builder::new()
                .name("research-log-io".into())
                .spawn_scoped(scope, move || action(engine))
                .map_err(|source| LogError::Io {
                    operation: "spawn isolated log worker",
                    source,
                })?;
            reader
                .join()
                .map_err(|_| LogError::EnginePanic { operation })?
        });
        if result
            .as_ref()
            .is_err_and(|error| !matches!(error, LogError::RecordTooLarge))
        {
            self.poisoned = true;
        }
        result
    }

    /// Published 0.4.2's Engine::sync uses an empty batch and can return without
    /// syncing. This real state/watermark write is deliberately nonempty and
    /// leaves the business position unchanged.
    pub fn sync(&mut self) -> LogResult<u64> {
        self.ensure_active()?;
        if self.head.synced_position == self.head.accepted_position {
            return Ok(self.head.synced_position);
        }
        let _reservation = self.account.maintenance().map_err(capacity_error)?;
        let mut next = self.head;
        next.synced_position = next.accepted_position;
        let mut batch = LogBatch::default();
        put_state(&mut batch, next)?;
        self.call("sync watermark", |engine| engine.write(&mut batch, true))?;
        self.head = next;
        Ok(next.synced_position)
    }

    /// Caller attests this prefix is recoverable from its authoritative SQL
    /// prefix/snapshot at the required durability. The log verifies bounds only.
    pub fn compact_prefix(&mut self, through: u64, sync: bool) -> LogResult<()> {
        self.ensure_active()?;
        if through < self.head.compacted_position || through > self.head.accepted_position {
            return Err(LogError::InvalidArgument(
                "compaction must be a monotonic accepted prefix",
            ));
        }
        if through == self.head.compacted_position {
            if sync {
                self.sync()?;
            }
            return Ok(());
        }
        let _reservation = self.account.maintenance().map_err(capacity_error)?;
        let mut next = self.head;
        next.compacted_position = through;
        if sync {
            next.synced_position = next.accepted_position;
        }
        let mut batch = LogBatch::default();
        batch.add_command(GROUP, Command::Compact { index: through + 1 });
        put_state(&mut batch, next)?;
        self.call("compact prefix", |engine| engine.write(&mut batch, sync))?;
        self.head = next;
        self.validate_indexes()
    }

    pub fn purge(&mut self) -> LogResult<Maintenance> {
        let _reservation = self.account.maintenance().map_err(capacity_error)?;
        let groups = self.purge_groups()?;
        let mut stats = Maintenance {
            compaction_suggested: groups.contains(&GROUP),
            bytes_on_disk: 0,
            logical_bytes: 0,
            files: 0,
        };
        let entries = fs::read_dir(&self.path).map_err(|source| LogError::Io {
            operation: "measure log files",
            source,
        })?;
        for entry in entries {
            let metadata =
                entry
                    .and_then(|entry| entry.metadata())
                    .map_err(|source| LogError::Io {
                        operation: "measure log file",
                        source,
                    })?;
            if !metadata.is_file() {
                continue;
            }
            stats.files += 1;
            stats.logical_bytes = stats.logical_bytes.saturating_add(metadata.len());
            #[cfg(unix)]
            let allocated = {
                use std::os::unix::fs::MetadataExt;
                metadata.blocks().saturating_mul(512)
            };
            #[cfg(not(unix))]
            let allocated = metadata.len();
            stats.bytes_on_disk = stats
                .bytes_on_disk
                .saturating_add(metadata.len().max(allocated));
        }
        Ok(stats)
    }

    /// Production maintenance uses the quota ledger, not directory inventory.
    pub(crate) fn reclaim_files(&mut self) -> LogResult<ExtentUsage> {
        let _reservation = self.account.maintenance().map_err(capacity_error)?;
        self.purge_groups()?;
        self.extent_usage()
    }
    fn purge_groups(&mut self) -> LogResult<Vec<u64>> {
        // Keep the published cache workaround for rewrite reads.
        self.isolated("purge", |engine| {
            engine
                .purge_expired_files()
                .map_err(|source| LogError::Engine {
                    operation: "purge",
                    source,
                })
        })
    }

    fn validate_indexes(&mut self) -> LogResult<()> {
        let first = self.call(
            "inspect first index",
            |engine| Ok(engine.first_index(GROUP)),
        )?;
        let last = self.call("inspect last index", |engine| Ok(engine.last_index(GROUP)))?;
        let valid = if self.head.accepted_position == self.head.compacted_position {
            first.is_none() && last.is_none()
        } else {
            first == Some(self.head.compacted_position + 1)
                && last == Some(self.head.accepted_position)
        };
        if !valid {
            self.poisoned = true;
            return Err(LogError::Corrupt(
                "entry range differs from runtime metadata",
            ));
        }
        let groups = self.call("inspect groups", |engine| Ok(engine.raft_groups()))?;
        if groups.iter().any(|group| *group != GROUP) {
            self.poisoned = true;
            return Err(LogError::Corrupt("foreign group in runtime log"));
        }
        Ok(())
    }

    fn ensure_active(&self) -> LogResult<()> {
        if self.poisoned {
            Err(LogError::Poisoned)
        } else {
            Ok(())
        }
    }

    fn call<T>(
        &mut self,
        operation: &'static str,
        action: impl FnOnce(&Engine<ExtentFileSystem<F>>) -> raft_engine::Result<T>,
    ) -> LogResult<T> {
        self.ensure_active()?;
        match catch_unwind(AssertUnwindSafe(|| action(&self.engine))) {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(source)) => {
                self.poisoned = true;
                Err(LogError::Engine { operation, source })
            }
            Err(_) => {
                self.poisoned = true;
                Err(LogError::EnginePanic { operation })
            }
        }
    }
}

fn capacity_error(source: std::io::Error) -> LogError {
    if let Some(quota) = source
        .get_ref()
        .and_then(|error| error.downcast_ref::<QuotaFailure>())
    {
        LogError::CapacityNotAccepted {
            used: quota.used,
            reserved: quota.reserved,
            requested: quota.requested,
            limit: quota.limit,
        }
    } else {
        LogError::Io {
            operation: "reserve managed log extent",
            source,
        }
    }
}

fn read_page<F: FileSystem>(
    engine: &Engine<F>,
    head: LogHead,
    start: u64,
    end: u64,
    max_bytes: usize,
) -> LogResult<Vec<LogRecord>> {
    let mut records = Vec::new();
    let mut bytes = 0usize;
    for position in start..end {
        let entry = engine
            .get_entry::<EnvelopeIndex>(GROUP, position)
            .map_err(|source| LogError::Engine {
                operation: "read entry",
                source,
            })?
            .ok_or(LogError::Corrupt("retained entry is missing"))?;
        if entry.schema_version != LOG_SCHEMA_VERSION
            || entry.position != position
            || entry.store_epoch != head.epoch.as_bytes()
        {
            return Err(LogError::Corrupt("entry schema, position or epoch differs"));
        }
        if entry.payload.len() > max_bytes.saturating_sub(bytes) {
            if records.is_empty() {
                return Err(LogError::RecordTooLarge);
            }
            break;
        }
        bytes += entry.payload.len();
        records.push(LogRecord {
            position,
            payload: entry.payload,
        });
    }
    Ok(records)
}

fn put_state(batch: &mut LogBatch, head: LogHead) -> LogResult<()> {
    let mut state = LogState::new();
    state.schema_version = LOG_SCHEMA_VERSION;
    state.store_epoch = head.epoch.as_bytes().to_vec();
    state.accepted_position = head.accepted_position;
    state.synced_position = head.synced_position;
    state.compacted_position = head.compacted_position;
    batch
        .put_message(GROUP, STATE_KEY.to_vec(), &state)
        .map_err(|source| LogError::Encoding {
            operation: "encode state",
            source,
        })
}

fn decode_state(state: LogState) -> LogResult<LogHead> {
    let epoch = Uuid::from_slice(&state.store_epoch)
        .map_err(|_| LogError::Corrupt("invalid store epoch"))?;
    if state.schema_version != LOG_SCHEMA_VERSION
        || epoch.is_nil()
        || state.accepted_position == u64::MAX
        || state.synced_position > state.accepted_position
        || state.compacted_position > state.accepted_position
    {
        return Err(LogError::Corrupt("invalid or unsupported runtime state"));
    }
    Ok(LogHead {
        epoch,
        accepted_position: state.accepted_position,
        synced_position: state.synced_position,
        compacted_position: state.compacted_position,
    })
}

fn create_private_directory(path: &Path) -> LogResult<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|source| LogError::Io {
        operation: "create log directory",
        source,
    })
}

#[cfg(test)]
mod tests;
