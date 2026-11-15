//! Handle-relative user source, authorized before probing or reading config.
use super::*;
use crate::agent::instructions::{InstructionRootIdentity, directory::Directory};
use std::{io::Read, path::Path};

/// Call on a blocking worker. No fallback discovery, creation or root guessing.
/// The native host supplies its app configuration root, not a model/IPC path.
pub fn load(
    root: &Path,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), HookConfigError>,
) -> Result<Option<HookConfiguration>, HookConfigError> {
    if !root.is_absolute() || root.canonicalize().map_err(|_| HookConfigError)? != root {
        return Err(HookConfigError);
    }
    let directory = Directory::root(root).map_err(|_| HookConfigError)?;
    let (device, inode) = directory.identity().map_err(|_| HookConfigError)?;
    authorize(&InstructionRootIdentity {
        canonical_path: root,
        device,
        inode,
    })?;
    let Some(directory) = directory
        .optional_child(Path::new("agent"))
        .map_err(|_| HookConfigError)?
    else {
        return Ok(None);
    };
    let Some(file) = directory
        .config_file("hooks.toml")
        .map_err(|_| HookConfigError)?
    else {
        return Ok(None);
    };
    if file.metadata().map_err(|_| HookConfigError)?.len() > 256 * 1024 {
        return Err(HookConfigError);
    }
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| HookConfigError)?;
    parse(std::str::from_utf8(&bytes).map_err(|_| HookConfigError)?).map(Some)
}
