use super::{
    LeaseError,
    storage::{self, Kind},
};
use crate::agent::instructions::directory::Directory;
use research_agent_core::workspace::{
    FileVersion, MAX_FILE_BYTES, MAX_FILES, MAX_WORKSPACE_BYTES, WorkspaceFile,
};
use sha2::{Digest, Sha256};
use std::{io::Read, os::unix::fs::MetadataExt, path::Path};

pub fn version(bytes: &[u8], executable: bool) -> FileVersion {
    FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable,
    }
}
pub fn scan(root: &Directory) -> Result<Vec<WorkspaceFile>, LeaseError> {
    let mut files = vec![];
    let mut budget = 20_000;
    let mut total = 0;
    collect(root, "", &mut files, &mut budget, &mut total)?;
    research_agent_core::workspace::validate_files(&files)?;
    Ok(files)
}
fn collect(
    root: &Directory,
    prefix: &str,
    files: &mut Vec<WorkspaceFile>,
    budget: &mut usize,
    total: &mut u64,
) -> Result<(), LeaseError> {
    for (name, kind) in storage::entries(root, budget)? {
        let name = name.to_str().ok_or(LeaseError::Invalid)?;
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        research_agent_core::workspace::validate_path(&path)?;
        match kind {
            Kind::Directory => {
                let child = root.child(Path::new(&name))?;
                if child.identity()?.0 != root.identity()?.0 {
                    return Err(LeaseError::Invalid);
                }
                collect(&child, &path, files, budget, total)?;
            }
            Kind::File => {
                if files.len() >= MAX_FILES {
                    return Err(LeaseError::Budget);
                }
                let mut file = root.config_file(name)?.ok_or(LeaseError::Conflict)?;
                let metadata = file.metadata()?;
                if metadata.nlink() != 1 {
                    return Err(LeaseError::Invalid);
                }
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(LeaseError::Budget);
                }
                let mut hash = Sha256::new();
                let mut size = 0u64;
                let mut buffer = [0u8; 65536];
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    size += n as u64;
                    *total = total.checked_add(n as u64).ok_or(LeaseError::Budget)?;
                    if size > MAX_FILE_BYTES || *total > MAX_WORKSPACE_BYTES {
                        return Err(LeaseError::Budget);
                    }
                    hash.update(&buffer[..n]);
                }
                files.push(WorkspaceFile {
                    path,
                    version: FileVersion {
                        sha256: format!("{:x}", hash.finalize()),
                        byte_size: size,
                        executable: metadata.mode() & 0o111 != 0,
                    },
                });
            }
            Kind::Other => return Err(LeaseError::Invalid),
        }
    }
    Ok(())
}
