//! Metadata publication after native byte capture. Caller owns the outer commit
//! and must durably stage/verify every referenced object before invoking this.
use super::execution_workspaces::{self, Binding, Error};
use research_agent_core::workspace::{FileChange, WorkspaceFile, WorkspaceManifest, compare};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Publish the complete inventory atomically within the caller's transaction.
/// A savepoint rolls back all proposal rows even if the caller commits after an
/// error. Object staging/rollback is the host's responsibility. This grants no
/// authority to apply the proposal, reactivate execution, or modify sources.
pub fn publish_in(
    tx: &mut Transaction<'_>,
    id: Uuid,
    binding: &Binding,
    changes: &[FileChange],
) -> Result<Uuid, Error> {
    if id.is_nil() || changes.is_empty() || changes.len() > 20_000 {
        return Err(Error::Conflict);
    }
    let baseline = execution_workspaces::read_binding_for_review(tx, binding)?;
    validate_inventory(&baseline, changes)?;
    let json = serde_json::to_vec(changes).map_err(|_| Error::Conflict)?;
    if json.len() > 32 * 1024 * 1024 {
        return Err(Error::Conflict);
    }
    let hash = format!("{:x}", Sha256::digest(json));
    // Metadata equality is necessary but not proof of file durability. The
    // native capture/store adapter verifies the actual bytes before publishing.
    for change in changes {
        for version in [&change.before, &change.after].into_iter().flatten() {
            let size: Option<u64> = tx
                .query_row(
                    "select byte_size from content_objects where sha256=?1",
                    [&version.sha256],
                    |r| r.get(0),
                )
                .optional()?;
            if size != Some(version.byte_size) {
                return Err(Error::Conflict);
            }
        }
    }
    let existing: Option<String> = tx.query_row(
        "select id from agent_change_sets where conversation_id=?1 and request_id=?2 and lease_id=?3 and inventory_hash=?4",
        params![binding.conversation_id.to_string(),binding.request_id.to_string(),binding.lease_id.to_string(),hash], |r|r.get(0),
    ).optional()?;
    if let Some(existing) = existing {
        verify_existing(tx, &existing, binding, changes)?;
        let id = Uuid::parse_str(&existing)
            .ok()
            .filter(|id| !id.is_nil())
            .ok_or(Error::Conflict)?;
        let (saved_binding, saved_baseline) =
            baseline::read(tx, binding.conversation_id, binding.manuscript_id, id)?;
        if saved_binding != *binding || saved_baseline != baseline {
            return Err(Error::Conflict);
        }
        return Ok(id);
    }
    let sp = tx.savepoint()?;
    sp.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version,baseline_json) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10)", params![id.to_string(),binding.conversation_id.to_string(),binding.request_id.to_string(),binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),binding.manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash,hash,execution_workspaces::encoded(&baseline)?.1])?;
    for change in changes {
        sp.execute("insert into agent_change_set_files(change_set_id,relative_path,before_sha256,after_sha256,before_executable,after_executable,source_json) values(?1,?2,?3,?4,?5,?6,?7)",params![id.to_string(),change.path,change.before.as_ref().map(|v|&v.sha256),change.after.as_ref().map(|v|&v.sha256),change.before.as_ref().map(|v|v.executable),change.after.as_ref().map(|v|v.executable),source_json(change)?])?;
    }
    sp.commit()?;
    Ok(id)
}
fn source_json(change: &FileChange) -> Result<Option<String>, Error> {
    change
        .source
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| Error::Conflict)
}
pub(crate) fn validate_inventory(
    baseline: &WorkspaceManifest,
    changes: &[FileChange],
) -> Result<(), Error> {
    let mut result: BTreeMap<_, _> = baseline
        .files
        .iter()
        .map(|file| (file.file.path.clone(), file.file.clone()))
        .collect();
    for change in changes {
        if let Some(after) = &change.after {
            result.insert(
                change.path.clone(),
                WorkspaceFile {
                    path: change.path.clone(),
                    version: after.clone(),
                },
            );
        } else {
            result.remove(&change.path);
        }
    }
    let canonical = compare(baseline, &result.into_values().collect::<Vec<_>>())
        .map_err(|_| Error::Conflict)?;
    if canonical != changes {
        return Err(Error::Conflict);
    }
    Ok(())
}
fn verify_existing(
    db: &Connection,
    id: &str,
    binding: &Binding,
    changes: &[FileChange],
) -> Result<(), Error> {
    let matches: bool = db.query_row("select exists(select 1 from agent_change_sets where id=?1 and turn_id=?2 and context_snapshot_id=?3 and manuscript_id=?4 and lease_id=?5 and manifest_hash=?6 and schema_version=1)",params![id,binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),binding.manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash],|r|r.get(0))?;
    let count: usize = db.query_row(
        "select count(*) from agent_change_set_files where change_set_id=?1",
        [id],
        |r| r.get(0),
    )?;
    if !matches || count != changes.len() {
        return Err(Error::Conflict);
    }
    for change in changes {
        let matches: bool = db.query_row("select exists(select 1 from agent_change_set_files where change_set_id=?1 and relative_path=?2 and before_sha256 is ?3 and after_sha256 is ?4 and before_executable is ?5 and after_executable is ?6 and source_json is ?7)",params![id,change.path,change.before.as_ref().map(|v|&v.sha256),change.after.as_ref().map(|v|&v.sha256),change.before.as_ref().map(|v|v.executable),change.after.as_ref().map(|v|v.executable),source_json(change)?],|r|r.get(0))?;
        if !matches {
            return Err(Error::Conflict);
        }
    }
    Ok(())
}

#[path = "change_sets/baseline.rs"]
mod baseline;
#[path = "change_sets/read.rs"]
mod read;
pub(crate) use read::read_file_operation_in;
pub use read::{ChangeSetStatus, StoredChangeSet, read_in};

/// Reject only an unapplied proposal. Retrying a confirmed rejection is safe;
/// other lifecycle states require their own recovery/undo operation. Caller
/// holds an immediate transaction, including the authorization read and commit.
pub fn reject_in(
    tx: &Transaction<'_>,
    conversation: Uuid,
    manuscript: Uuid,
    id: Uuid,
) -> Result<StoredChangeSet, Error> {
    let mut proposal = read_in(tx, conversation, manuscript, id)?;
    match proposal.status {
        ChangeSetStatus::Rejected => return Ok(proposal),
        ChangeSetStatus::Pending => {}
        _ => return Err(Error::Conflict),
    }
    let changed = tx.execute(
        "update agent_change_sets set status='rejected' where id=?1 and status='pending'",
        [id.to_string()],
    )?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    proposal.status = ChangeSetStatus::Rejected;
    Ok(proposal)
}
