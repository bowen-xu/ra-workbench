//! Explicit manuscript instruction trust. Blocking storage operations; the host
//! must derive root identity from an opened directory, never from frontend fields.
use crate::agent::instructions::InstructionRootIdentity;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TrustError {
    #[error("instruction trust unavailable")]
    Storage,
    #[error("instruction trust changed")]
    Conflict,
    #[error("manuscript is not active")]
    Inactive,
    #[error("invalid instruction root")]
    Invalid,
}
#[derive(Debug, PartialEq, Eq)]
pub struct TrustState {
    pub revision: u64,
    pub granted: bool,
    pub canonical_path: String,
    pub device: u64,
    pub inode: u64,
}
fn active(db: &Connection, manuscript: Uuid) -> Result<(), TrustError> {
    let exists: bool = db
        .query_row(
            "select exists(select 1 from manuscripts where id=?1 and status='active')",
            [manuscript.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| TrustError::Storage)?;
    if exists {
        Ok(())
    } else {
        Err(TrustError::Inactive)
    }
}
pub fn read(db: &Connection, manuscript: Uuid) -> Result<Option<TrustState>, TrustError> {
    active(db, manuscript)?;
    db.query_row("select revision,granted,canonical_path,device,inode from agent_manuscript_instruction_trust where manuscript_id=?1", [manuscript.to_string()], |row| {
        let device: String = row.get(3)?;
        let inode: String = row.get(4)?;
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, device, inode))
    }).optional().map_err(|_| TrustError::Storage)?.map(|(revision, granted, canonical_path, device, inode)| {
        Ok(TrustState { revision, granted, canonical_path, device: device.parse().map_err(|_| TrustError::Storage)?, inode: inode.parse().map_err(|_| TrustError::Storage)? })
    }).transpose()
}
/// Absence is untrusted. Compare both resource binding and opened root identity.
/// Call within the host's read transaction so revocation and materialization are
/// serialized with the snapshot decision; this does not grant model disclosure.
pub fn is_trusted(
    db: &Connection,
    manuscript: Uuid,
    root: &InstructionRootIdentity<'_>,
) -> Result<bool, TrustError> {
    let Some(state) = read(db, manuscript)? else {
        return Ok(false);
    };
    Ok(state.granted
        && Some(state.canonical_path.as_str()) == root.canonical_path.to_str()
        && state.device == root.device
        && state.inode == root.inode)
}
/// Only an explicit host user action may call this. Revision zero denotes no row.
/// Revocation retains a tombstone revision to prevent stale grants being replayed.
pub fn update(
    db: &mut Connection,
    manuscript: Uuid,
    expected_revision: u64,
    root: &InstructionRootIdentity<'_>,
    granted: bool,
) -> Result<TrustState, TrustError> {
    let path = root
        .canonical_path
        .to_str()
        .filter(|value| !value.is_empty() && !value.contains('\0'))
        .ok_or(TrustError::Invalid)?;
    if !root.canonical_path.is_absolute() || expected_revision >= 9_007_199_254_740_991 {
        return Err(TrustError::Invalid);
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| TrustError::Storage)?;
    let current = read(&tx, manuscript)?;
    if current.as_ref().map_or(0, |state| state.revision) != expected_revision {
        return Err(TrustError::Conflict);
    }
    let revision = expected_revision + 1;
    tx.execute("insert into agent_manuscript_instruction_trust(manuscript_id,revision,granted,canonical_path,device,inode) values (?1,?2,?3,?4,?5,?6) on conflict(manuscript_id) do update set revision=excluded.revision,granted=excluded.granted,canonical_path=excluded.canonical_path,device=excluded.device,inode=excluded.inode,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![manuscript.to_string(), revision, granted, path, root.device.to_string(), root.inode.to_string()]).map_err(|_| TrustError::Storage)?;
    tx.commit().map_err(|_| TrustError::Storage)?;
    Ok(TrustState {
        revision,
        granted,
        canonical_path: path.into(),
        device: root.device,
        inode: root.inode,
    })
}

#[cfg(test)]
#[path = "instruction_trust/tests.rs"]
mod tests;

/// Revocation must remain possible even when the filesystem root is unavailable.
pub fn revoke(
    db: &mut Connection,
    manuscript: Uuid,
    expected_revision: u64,
) -> Result<u64, TrustError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| TrustError::Storage)?;
    let state = read(&tx, manuscript)?.ok_or(TrustError::Conflict)?;
    if state.revision != expected_revision || expected_revision >= 9_007_199_254_740_991 {
        return Err(TrustError::Conflict);
    }
    let revision = expected_revision + 1;
    tx.execute("update agent_manuscript_instruction_trust set granted=0,revision=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where manuscript_id=?1", params![manuscript.to_string(), revision]).map_err(|_| TrustError::Storage)?;
    tx.commit().map_err(|_| TrustError::Storage)?;
    Ok(revision)
}
