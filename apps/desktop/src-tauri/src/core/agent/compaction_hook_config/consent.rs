//! App-owned bridge from durable consent to synchronous invocation checks.
//! Construct once per app database and share it with all native consent writers.
//! Direct SQL writers and other processes are not live-synchronized by this port.
use crate::agent::{
    instructions::InstructionRootIdentity,
    repository::compaction_hook_trust::{self as repository, GrantKey, GrantState, TrustError},
};
use crate::data_interface::{IntoDatabase, SharedDatabase};
use rusqlite::Connection;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const MAX_LOADED_GRANTS: usize = 4096;

/// A prepared owner retains this handle instead of a detached GrantState.
pub struct LiveHookGrant {
    key: GrantKey,
    writer: Mutex<()>,
    state: Mutex<Option<GrantState>>,
    ready: AtomicBool,
}
impl LiveHookGrant {
    /// No I/O or waiting. Pending/failed updates and contention deny admission.
    /// Revocation cannot cancel a process already admitted by an earlier check.
    pub fn allows(
        &self,
        key: &GrantKey,
        root: &InstructionRootIdentity<'_>,
        configuration_revision: &str,
    ) -> bool {
        if !self.ready.load(Ordering::Acquire) || key != &self.key {
            return false;
        }
        let Ok(state) = self.state.try_lock() else {
            return false;
        };
        state
            .as_ref()
            .is_some_and(|state| state.allows(key, root, configuration_revision))
            && self.ready.load(Ordering::Acquire)
    }
}

/// Blocking native storage adapter; use on the configuration/consent worker.
/// This is a bounded permission cache, not a task or cancellation registry.
pub struct SqliteHookConsent {
    db: SharedDatabase,
    loaded: Mutex<Vec<Arc<LiveHookGrant>>>,
    capacity: usize,
}
impl SqliteHookConsent {
    pub(crate) fn new(db: impl IntoDatabase) -> Self {
        Self {
            db: db.into_database(),
            loaded: Mutex::new(Vec::new()),
            capacity: MAX_LOADED_GRANTS,
        }
    }

    fn slot(&self, key: &GrantKey) -> Result<Arc<LiveHookGrant>, TrustError> {
        let mut loaded = self.loaded.lock().map_err(|_| TrustError::Storage)?;
        if let Some(slot) = loaded.iter().find(|slot| &slot.key == key) {
            return Ok(Arc::clone(slot));
        }
        if loaded.len() >= self.capacity {
            loaded.retain(|slot| Arc::strong_count(slot) > 1);
        }
        if loaded.len() >= self.capacity {
            return Err(TrustError::Storage);
        }
        let slot = Arc::new(LiveHookGrant {
            key: key.clone(),
            writer: Mutex::new(()),
            state: Mutex::new(None),
            ready: AtomicBool::new(false),
        });
        loaded.push(Arc::clone(&slot));
        Ok(slot)
    }

    fn apply(
        &self,
        key: &GrantKey,
        operation: impl FnOnce(&mut Connection) -> Result<Option<GrantState>, TrustError>,
    ) -> Result<(Arc<LiveHookGrant>, Option<GrantState>), TrustError> {
        let slot = self.slot(key)?;
        // Writers serialize per scope; readers never acquire this worker gate.
        let _writer = slot.writer.lock().map_err(|_| TrustError::Storage)?;
        slot.ready.store(false, Ordering::Release);
        let result = self
            .db
            .lock()
            .map_err(|_| TrustError::Storage)
            .and_then(|mut db| operation(&mut db));
        // Never hold a reader-facing lock during SQLite work. A failed write
        // preserves the durable row, but live admission stays closed until load.
        *slot.state.lock().map_err(|_| TrustError::Storage)? =
            result.as_ref().ok().cloned().flatten();
        slot.ready.store(result.is_ok(), Ordering::Release);
        let state = result?;
        drop(_writer);
        Ok((slot, state))
    }

    pub fn load(&self, key: &GrantKey) -> Result<Arc<LiveHookGrant>, TrustError> {
        self.apply(key, |db| repository::read(db, key))
            .map(|(slot, _)| slot)
    }

    /// Blocking settings read; publishes the verified durable state to owners.
    pub fn read(&self, key: &GrantKey) -> Result<Option<GrantState>, TrustError> {
        self.apply(key, |db| repository::read(db, key))
            .map(|(_, state)| state)
    }

    /// Explicit native user decision only; parsing a file never calls this.
    pub fn update(
        &self,
        key: &GrantKey,
        expected_revision: u64,
        root: &InstructionRootIdentity<'_>,
        configuration_revision: &str,
        granted: bool,
    ) -> Result<GrantState, TrustError> {
        self.apply(key, |db| {
            repository::update(
                db,
                key,
                expected_revision,
                root,
                configuration_revision,
                granted,
            )
            .map(Some)
        })?
        .1
        .ok_or(TrustError::Storage)
    }

    pub fn revoke(&self, key: &GrantKey, expected_revision: u64) -> Result<GrantState, TrustError> {
        self.apply(key, |db| {
            repository::revoke(db, key, expected_revision).map(Some)
        })?
        .1
        .ok_or(TrustError::Storage)
    }
}

#[cfg(test)]
#[path = "consent/tests.rs"]
mod tests;
