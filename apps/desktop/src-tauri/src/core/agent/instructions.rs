//! Bounded native instruction discovery. Call only on a blocking worker after
//! the host authorizes this execution environment; paths are not IPC grants.
// Adapted from OpenAI Codex core/src/agents_md.rs, e363b08c9175ac1cbe5893615dd2cb9ddf95043b.
// Copyright 2025 OpenAI. Apache-2.0; see packages/agent-core/third-party/codex/.
// Changes: explicit authorized root, reject links, bounded reads, content hashes.
use research_agent_core::instructions::{InstructionDocument, candidate_filenames};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path},
};

#[path = "instructions/directory.rs"]
pub(crate) mod directory;
use directory::Directory;

const MAX_DEPTH: usize = 64;
const MAX_READ_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InstructionReadError {
    #[error("invalid instruction scope")]
    InvalidScope,
    #[error("instruction root is not authorized")]
    Unauthorized,
    #[error("instruction source unavailable")]
    Unavailable,
    #[error("instruction source exceeds read limit")]
    Budget,
    #[error("secure instruction reads are unavailable on this platform")]
    UnsupportedPlatform,
    #[error("instruction source is not UTF-8")]
    Encoding,
}

/// Identity of the opened root, used by host trust policy. Device/inode are
/// native-only evidence, not a persistent grant or a frontend capability token.
pub struct InstructionRootIdentity<'a> {
    pub canonical_path: &'a Path,
    pub device: u64,
    pub inode: u64,
}

/// `root` must be a host-authorized canonical directory, not a model/frontend
/// supplied path. `cwd` is relative to root; empty means root. No ancestor above
/// root is inspected. This does not grant permission to send contents to a model.
/// Unix reads pin directory handles and reject symlinks at every component.
/// Hosts must still coordinate content writes; this is not an atomic file snapshot.
/// Other platforms fail explicitly until an equivalent handle adapter is provided.
/// `authorize` runs on the opened root before any child lookup or content read.
/// It must verify the host's current trust policy, including grant revocation.
pub fn read_project_instructions(
    root: &Path,
    cwd: &Path,
    environment_id: &str,
    fallbacks: &[String],
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<Vec<InstructionDocument>, InstructionReadError> {
    read_project_instruction_scopes(root, &[cwd], environment_id, fallbacks, authorize)
}

/// Load the union of selected scopes once, with one shared read budget.
pub fn read_project_instruction_scopes(
    root: &Path,
    scopes: &[&Path],
    environment_id: &str,
    fallbacks: &[String],
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<Vec<InstructionDocument>, InstructionReadError> {
    read_project_instruction_scopes_with_fallbacks(
        root,
        scopes,
        environment_id,
        |_| Ok(fallbacks.to_vec()),
        authorize,
    )
}

/// Resolve candidate names for each directory independently. The caller supplies
/// only already-validated configuration; handles and the union read budget remain
/// owned by the same discovery implementation.
pub(crate) fn read_project_instruction_scopes_with_fallbacks(
    root: &Path,
    scopes: &[&Path],
    environment_id: &str,
    fallbacks: impl Fn(&Path) -> Result<Vec<String>, InstructionReadError>,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<Vec<InstructionDocument>, InstructionReadError> {
    if environment_id.trim().is_empty()
        || !root.is_absolute()
        || fs::canonicalize(root).map_err(|_| InstructionReadError::Unavailable)? != root
    {
        return Err(InstructionReadError::InvalidScope);
    }
    if scopes.is_empty() || scopes.len() > 256 {
        return Err(InstructionReadError::InvalidScope);
    }
    for cwd in scopes {
        if cwd.components().count() > MAX_DEPTH
            || cwd.components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(InstructionReadError::InvalidScope);
        }
    }
    let root_handle = Directory::root(root)?;
    let (device, inode) = root_handle.identity()?;
    authorize(&InstructionRootIdentity {
        canonical_path: root,
        device,
        inode,
    })?;
    let mut directories = vec![(root.to_path_buf(), root_handle)];
    let mut indices = std::collections::HashMap::from([(root.to_path_buf(), 0usize)]);
    for cwd in scopes {
        let mut parent_index = 0;
        for component in cwd.components() {
            let (path, parent) = &directories[parent_index];
            let path = path.join(component);
            if let Some(index) = indices.get(&path) {
                parent_index = *index;
                continue;
            }
            if directories.len() >= 256 {
                return Err(InstructionReadError::Budget);
            }
            let child = parent.child(Path::new(component.as_os_str()))?;
            parent_index = directories.len();
            indices.insert(path.clone(), parent_index);
            directories.push((path, child));
        }
    }
    let mut documents = Vec::new();
    let mut remaining = MAX_READ_BYTES;
    for (directory, handle) in directories {
        let scope = directory
            .strip_prefix(root)
            .map_err(|_| InstructionReadError::InvalidScope)?;
        let fallbacks = fallbacks(scope)?;
        let names =
            candidate_filenames(&fallbacks).map_err(|_| InstructionReadError::InvalidScope)?;
        for name in &names {
            let path = directory.join(name);
            let Some(file) = handle.file(name)? else {
                continue;
            };
            let metadata = file
                .metadata()
                .map_err(|_| InstructionReadError::Unavailable)?;
            if metadata.len() > remaining as u64 {
                return Err(InstructionReadError::Budget);
            }
            let mut bytes = Vec::new();
            file.take(remaining as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| InstructionReadError::Unavailable)?;
            if bytes.len() > remaining {
                return Err(InstructionReadError::Budget);
            }
            remaining -= bytes.len();
            let revision = format!("sha256:{:x}", Sha256::digest(&bytes));
            let text = String::from_utf8(bytes).map_err(|_| InstructionReadError::Encoding)?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| InstructionReadError::InvalidScope)?;
            let scope = directory
                .strip_prefix(root)
                .map_err(|_| InstructionReadError::InvalidScope)?;
            let scope = scope.to_str().ok_or(InstructionReadError::InvalidScope)?;
            documents.push(InstructionDocument {
                environment_id: environment_id.to_owned(),
                scope_id: if scope.is_empty() {
                    ".".into()
                } else {
                    scope.into()
                },
                source_id: relative
                    .to_str()
                    .ok_or(InstructionReadError::InvalidScope)?
                    .into(),
                revision,
                text,
            });
            break;
        }
    }
    Ok(documents)
}

#[cfg(all(test, unix))]
#[path = "instructions/tests.rs"]
mod tests;

/// Inspect a pinned native root without enumerating children or reading content.
/// The host uses this to bind an explicit user trust action to the opened object.
pub fn inspect_instruction_root<T>(
    root: &Path,
    inspect: impl FnOnce(&InstructionRootIdentity<'_>) -> T,
) -> Result<T, InstructionReadError> {
    if !root.is_absolute()
        || fs::canonicalize(root).map_err(|_| InstructionReadError::Unavailable)? != root
    {
        return Err(InstructionReadError::InvalidScope);
    }
    let directory = Directory::root(root)?;
    let (device, inode) = directory.identity()?;
    Ok(inspect(&InstructionRootIdentity {
        canonical_path: root,
        device,
        inode,
    }))
}
