//! Handle-relative publication reuses the workspace's no-follow file helpers.
use super::*;
use crate::agent::{instructions::directory::Directory, workspace::storage};
use nix::fcntl::AtFlags;
use std::{os::unix::fs::MetadataExt, path::Path};

pub(super) fn save(root: &Path, target: &Path, bytes: &[u8]) -> Result<(), HookContextError> {
    let directory = Directory::root(root).map_err(|_| HookContextError)?;
    let name = target
        .strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .filter(|name| !name.is_empty() && !name.contains(['/', '\\', '\0']))
        .ok_or(HookContextError)?;
    save_opened(root, &directory, name, bytes)
}

fn save_opened(
    root: &Path,
    directory: &Directory,
    name: &str,
    bytes: &[u8],
) -> Result<(), HookContextError> {
    let temporary = format!(".hook-output-{}.tmp", uuid::Uuid::new_v4());
    let file =
        storage::create_file(directory, &temporary, bytes, false).map_err(|_| HookContextError)?;
    // Publish complete, synced bytes without overwriting an existing entry. All
    // names remain relative to the held root even if an ancestor is replaced.
    let publication = nix::unistd::linkat(
        directory,
        temporary.as_str(),
        directory,
        name,
        AtFlags::empty(),
    );
    // A name collision must not leave another copy of the full output behind.
    let cleanup = storage::unlink_file(directory, temporary.as_ref());
    publication.map_err(|_| HookContextError)?;
    cleanup.map_err(|_| HookContextError)?;
    let published = directory
        .config_file(name)
        .map_err(|_| HookContextError)?
        .ok_or(HookContextError)?;
    let actual = published.metadata().map_err(|_| HookContextError)?;
    let held = file.metadata().map_err(|_| HookContextError)?;
    if (actual.dev(), actual.ino()) != (held.dev(), held.ino()) {
        return Err(HookContextError);
    }
    storage::sync(directory).map_err(|_| HookContextError)?;
    // A pathname for the old opened directory is no longer a recovery locator
    // after replacement. Never publish it into the compaction context.
    if Directory::root(root)
        .and_then(|current| current.identity())
        .map_err(|_| HookContextError)?
        != directory.identity().map_err(|_| HookContextError)?
    {
        return Err(HookContextError);
    }
    Ok(())
}

#[cfg(test)]
#[path = "unix/tests.rs"]
mod tests;
