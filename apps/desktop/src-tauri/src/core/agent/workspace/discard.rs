//! A parent-side journal survives removal of the lease's own metadata. This is
//! native storage metadata and must never be included in the writable sandbox.
use super::{LeaseError, WorkspaceStore, storage};
use crate::agent::instructions::directory::Directory;
use research_agent_core::workspace::WorkspaceIdentity;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Seek},
    os::unix::fs::MetadataExt,
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardState {
    Pending,
    Complete,
}

pub(super) fn state(
    store: &WorkspaceStore,
    expected: &WorkspaceIdentity,
) -> Result<Option<DiscardState>, LeaseError> {
    for (name, state) in [
        (receipt_name(expected), DiscardState::Complete),
        (journal_name(expected), DiscardState::Pending),
    ] {
        if let Some(mut file) = store.root.config_file(&name)? {
            if read_deletion(&mut file)?.identity != *expected {
                return Err(LeaseError::Conflict);
            }
            return Ok(Some(state));
        }
    }
    Ok(None)
}

/// Caller retains the lease ownership lock until the durable parent journal exists.
pub(super) fn mark(
    root: &Directory,
    directory: &Directory,
    identity: &WorkspaceIdentity,
) -> Result<(), LeaseError> {
    storage::same_child(root, OsStr::new(&identity.lease_id.to_string()), directory)?;
    if directory.config_file(super::execution::MARKER)?.is_some()
        || root.config_file(&journal_name(identity))?.is_some()
        || root.config_file(&receipt_name(identity))?.is_some()
    {
        return Err(LeaseError::Conflict);
    }
    let (device, inode) = directory.identity()?;
    let bytes = serde_json::to_vec(&Deletion {
        identity: identity.clone(),
        device,
        inode,
    })
    .map_err(|_| LeaseError::Invalid)?;
    storage::create_file(root, &journal_name(identity), &bytes, false)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Deletion {
    identity: WorkspaceIdentity,
    device: u64,
    inode: u64,
}
pub(super) fn journal_name(identity: &WorkspaceIdentity) -> String {
    format!("deleting-{}.json", identity.lease_id)
}
pub(super) fn receipt_name(identity: &WorkspaceIdentity) -> String {
    format!("discarded-{}.json", identity.lease_id)
}
fn read_deletion(file: &mut File) -> Result<Deletion, LeaseError> {
    file.rewind()?;
    let mut bytes = vec![];
    file.take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err(LeaseError::Budget);
    }
    let deletion: Deletion = serde_json::from_slice(&bytes).map_err(|_| LeaseError::Invalid)?;
    research_agent_core::workspace::WorkspaceManifest {
        schema_version: 1,
        identity: deletion.identity.clone(),
        files: vec![],
    }
    .validate()?;
    Ok(deletion)
}
fn lock(file: &File) -> Result<(), LeaseError> {
    fs2::FileExt::try_lock_exclusive(file).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            LeaseError::Busy
        } else {
            LeaseError::Unavailable
        }
    })
}
pub(super) fn discard(
    store: &WorkspaceStore,
    expected: &WorkspaceIdentity,
) -> Result<(), LeaseError> {
    if expected.lease_id.is_nil() {
        return Err(LeaseError::Invalid);
    }
    let completed = receipt_name(expected);
    if let Some(mut receipt) = store.root.config_file(&completed)? {
        let deletion = read_deletion(&mut receipt)?;
        if &deletion.identity != expected
            || store
                .root
                .optional_child(Path::new(&expected.lease_id.to_string()))?
                .is_some()
        {
            // A completed operation must never delete a replacement, even if the
            // filesystem has recycled the original inode number.
            return Err(LeaseError::Conflict);
        }
        return storage::sync(&store.root);
    }
    let name = journal_name(expected);
    let mut owner = None;
    let mut journal = match store.root.config_file(&name)? {
        Some(file) => file,
        None => {
            let (directory, held, _) = store.open_owned(expected)?;
            owner = Some(held);
            let (device, inode) = directory.identity()?;
            let bytes = serde_json::to_vec(&Deletion {
                identity: expected.clone(),
                device,
                inode,
            })
            .map_err(|_| LeaseError::Invalid)?;
            storage::create_file(&store.root, &name, &bytes, false)?
        }
    };
    lock(&journal)?;
    let metadata = journal.metadata()?;
    let deletion = read_deletion(&mut journal)?;
    if &deletion.identity != expected {
        return Err(LeaseError::Conflict);
    }
    let lease_name = expected.lease_id.to_string();
    if let Some(directory) = store.root.optional_child(Path::new(&lease_name))? {
        if directory.identity()? != (deletion.device, deletion.inode) {
            return Err(LeaseError::Conflict);
        }
        if owner.is_none()
            && let Some(file) = directory.config_file("owner.lock")?
        {
            lock(&file)?;
            owner = Some(super::lease::OwnershipLock(file, Default::default()));
        }
        if directory.config_file(super::execution::MARKER)?.is_some() {
            return Err(LeaseError::Incomplete);
        }
        storage::remove_tree(&store.root, OsStr::new(&lease_name), &mut 100_000, 0)?;
    }
    // Keep both ownership locks alive until removal has finished. Detect a
    // replaced journal before publishing its receipt; the parent is app-private.
    let current = store
        .root
        .config_file(&name)?
        .ok_or(LeaseError::Conflict)?
        .metadata()?;
    if (current.dev(), current.ino()) != (metadata.dev(), metadata.ino()) {
        return Err(LeaseError::Conflict);
    }
    // Preserve the already fsynced journal as an immutable completion receipt.
    // Rename within the private directory is atomic; no partial receipt is created.
    // The exclusive journal lock serializes publishers of this lease identity.
    if store.root.config_file(&completed)?.is_some() {
        return Err(LeaseError::Conflict);
    }
    nix::fcntl::renameat(
        &*store.root,
        OsStr::new(&name),
        &*store.root,
        OsStr::new(&completed),
    )
    .map_err(|_| LeaseError::Unavailable)?;
    storage::sync(&store.root)?;
    drop(owner);
    Ok(())
}
