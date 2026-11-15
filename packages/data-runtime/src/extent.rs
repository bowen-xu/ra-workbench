//! Managed engine-file extent accounting over the published FileSystem port.
//! This is an in-memory quota, not a journal or volume-wide free-space promise.
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

mod file_system;
pub(crate) use file_system::ExtentFileSystem;

/// Separate from Engine::get_used_size, which is file-count × target size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentUsage {
    pub managed_file_extent_bytes: u64,
    pub reserved_bytes: u64,
    pub hard_limit_bytes: u64,
    pub maintenance_reserve_bytes: u64,
    pub managed_files: usize,
}
#[derive(Clone, Copy)]
pub(crate) enum ReservationKind {
    Admission,
    Maintenance,
}
#[derive(Debug, thiserror::Error)]
#[error("log_extent_quota: used={used}, reserved={reserved}, requested={requested}, limit={limit}")]
pub(crate) struct QuotaFailure {
    pub used: u64,
    pub reserved: u64,
    pub requested: u64,
    pub limit: u64,
}

// Directory locks and engine control files bypass FileSystem::Writer. The
// current fixed configuration has one directory; reserve fixed control slack.
const CONTROL_BYTES: u64 = 64 * 1024;
struct FileInfo {
    extent: u64,
    linked: bool,
    leases: usize,
}
struct State {
    paths: HashMap<PathBuf, u64>,
    files: HashMap<u64, FileInfo>,
    next: u64,
    used: u64,
    reservation: Option<(u64, ReservationKind)>,
}
pub(crate) struct Account {
    state: Mutex<State>,
    hard: u64,
    maintenance: u64,
}
pub(crate) struct FileLease {
    account: Arc<Account>,
    key: u64,
}
pub(crate) struct Reservation {
    account: Arc<Account>,
}
impl Account {
    pub(crate) fn inventory(root: &Path, hard: u64, maintenance: u64) -> io::Result<Arc<Self>> {
        let mut state = State {
            paths: HashMap::new(),
            files: HashMap::new(),
            next: 1,
            used: 0,
            reservation: None,
        };
        match fs::read_dir(root) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let metadata = entry.path().symlink_metadata()?;
                    if !metadata.is_file() {
                        return Err(io::Error::other(
                            "log extent inventory requires regular engine files",
                        ));
                    }
                    let key = state.next;
                    state.next += 1;
                    state.used = state
                        .used
                        .checked_add(metadata.len())
                        .ok_or_else(unavailable)?;
                    state.paths.insert(entry.path(), key);
                    state.files.insert(
                        key,
                        FileInfo {
                            extent: metadata.len(),
                            linked: true,
                            leases: 0,
                        },
                    );
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if state.used.saturating_add(CONTROL_BYTES) > hard {
            return Err(quota(&state, state.used, hard));
        }
        Ok(Arc::new(Self {
            state: Mutex::new(state),
            hard,
            maintenance,
        }))
    }
    pub(crate) fn usage(&self) -> io::Result<ExtentUsage> {
        let state = self.state.lock().map_err(|_| unavailable())?;
        Ok(ExtentUsage {
            managed_file_extent_bytes: state.used,
            reserved_bytes: CONTROL_BYTES + state.reservation.map_or(0, |v| v.0),
            hard_limit_bytes: self.hard,
            maintenance_reserve_bytes: self.maintenance,
            managed_files: state.files.len(),
        })
    }
    pub(crate) fn reserve(
        self: &Arc<Self>,
        bytes: u64,
        kind: ReservationKind,
    ) -> io::Result<Reservation> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if state.reservation.is_some() {
            return Err(io::Error::other("nested log extent reservation"));
        }
        let limit = match kind {
            ReservationKind::Admission => self.hard - self.maintenance,
            ReservationKind::Maintenance => self.hard,
        };
        if state
            .used
            .saturating_add(CONTROL_BYTES)
            .saturating_add(bytes)
            > limit
        {
            return Err(quota(&state, bytes, limit));
        }
        state.reservation = Some((bytes, kind));
        Ok(Reservation {
            account: self.clone(),
        })
    }
    pub(crate) fn maintenance(self: &Arc<Self>) -> io::Result<Reservation> {
        let usage = self.usage()?;
        self.reserve(
            self.hard
                .saturating_sub(usage.managed_file_extent_bytes)
                .saturating_sub(CONTROL_BYTES),
            ReservationKind::Maintenance,
        )
    }
    pub(crate) fn acquire(
        self: &Arc<Self>,
        path: &Path,
        actual: u64,
    ) -> io::Result<Arc<FileLease>> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let key = if let Some(key) = state.paths.get(path) {
            *key
        } else {
            let key = state.next;
            state.next = state.next.checked_add(1).ok_or_else(unavailable)?;
            state.paths.insert(path.to_owned(), key);
            state.files.insert(
                key,
                FileInfo {
                    extent: 0,
                    linked: true,
                    leases: 0,
                },
            );
            key
        };
        grow(&mut state, key, actual, self.hard, self.maintenance)?;
        state.files.get_mut(&key).ok_or_else(unavailable)?.leases += 1;
        Ok(Arc::new(FileLease {
            account: self.clone(),
            key,
        }))
    }
    pub(crate) fn unlink(&self, path: &Path) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if let Some(key) = state.paths.remove(path) {
            let info = state.files.get_mut(&key).ok_or_else(unavailable)?;
            info.linked = false;
            if info.leases == 0 {
                remove(&mut state, key);
            }
        }
        Ok(())
    }
    pub(crate) fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if let Some(key) = state.paths.remove(from) {
            if state.paths.contains_key(to) {
                return Err(io::Error::other(
                    "log extent rename replaced a tracked file",
                ));
            }
            state.paths.insert(to.to_owned(), key);
        }
        Ok(())
    }
}
impl FileLease {
    pub(crate) fn grow(&self, end: u64) -> io::Result<()> {
        let mut state = self.account.state.lock().map_err(|_| unavailable())?;
        grow(
            &mut state,
            self.key,
            end,
            self.account.hard,
            self.account.maintenance,
        )
    }
    pub(crate) fn truncate(&self, end: u64) -> io::Result<()> {
        let mut state = self.account.state.lock().map_err(|_| unavailable())?;
        let old = state.files.get(&self.key).ok_or_else(unavailable)?.extent;
        if end > old {
            return grow(
                &mut state,
                self.key,
                end,
                self.account.hard,
                self.account.maintenance,
            );
        }
        state.used -= old - end;
        state
            .files
            .get_mut(&self.key)
            .ok_or_else(unavailable)?
            .extent = end;
        Ok(())
    }
}
impl Drop for FileLease {
    fn drop(&mut self) {
        // A poisoned account cannot admit more data; retaining its charge is
        // safer than trying to repair accounting from a destructor.
        if let Ok(mut state) = self.account.state.lock()
            && let Some(info) = state.files.get_mut(&self.key)
        {
            info.leases = info.leases.saturating_sub(1);
            if !info.linked && info.leases == 0 {
                remove(&mut state, self.key);
            }
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.account.state.lock() {
            state.reservation = None;
        }
    }
}
fn grow(state: &mut State, key: u64, end: u64, hard: u64, maintenance: u64) -> io::Result<()> {
    let old = state.files.get(&key).ok_or_else(unavailable)?.extent;
    if end <= old {
        return Ok(());
    }
    let delta = end - old;
    let (reserved, kind) = state
        .reservation
        .unwrap_or((0, ReservationKind::Maintenance));
    let consumed = delta.min(reserved);
    let limit = match kind {
        ReservationKind::Admission => hard - maintenance,
        ReservationKind::Maintenance => hard,
    };
    if state
        .used
        .saturating_add(CONTROL_BYTES)
        .saturating_add(delta)
        .saturating_add(reserved - consumed)
        > limit
    {
        return Err(quota(state, delta, limit));
    }
    state.used += delta;
    state.files.get_mut(&key).ok_or_else(unavailable)?.extent = end;
    if state.reservation.is_some() {
        state.reservation = Some((reserved - consumed, kind));
    }
    Ok(())
}
fn remove(state: &mut State, key: u64) {
    if let Some(info) = state.files.remove(&key) {
        state.used -= info.extent;
    }
}
fn quota(state: &State, requested: u64, limit: u64) -> io::Error {
    io::Error::other(QuotaFailure {
        used: state.used,
        reserved: CONTROL_BYTES + state.reservation.map_or(0, |v| v.0),
        requested,
        limit,
    })
}
fn unavailable() -> io::Error {
    io::Error::other("log extent account unavailable")
}

#[cfg(test)]
mod tests;
