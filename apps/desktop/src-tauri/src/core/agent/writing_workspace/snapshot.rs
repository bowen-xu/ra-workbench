//! Original bounded execution-copy capture. Domain selection supplies registered rows.
//! Core retains no-follow file reads, buffers, hashes and workspace budgets.
use super::{WritingWorkspaceRequest, WritingWorkspaceSnapshot};
use crate::agent::{
    instructions::directory::Directory, workspace::WorkspaceInput,
    writing_context::WritingContextError as Error,
};
use research_agent_core::workspace::{
    BaselineFile, FileVersion, MAX_FILE_BYTES, MAX_FILES, MAX_WORKSPACE_BYTES, SourceVersion,
    WorkspaceFile, WorkspaceIdentity, WorkspaceManifest,
};
use research_workbench_native_sdk::writing_project::ManuscriptFile;
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

pub(crate) fn validate_scope(scope: &WorkspaceIdentity) -> Result<(), Error> {
    WorkspaceManifest {
        schema_version: 1,
        identity: scope.clone(),
        files: vec![],
    }
    .validate()
    .map_err(|_| Error::InvalidInput)?;
    Ok(())
}

pub(crate) fn from_files(
    manuscript_root: &Path,
    scope: WorkspaceIdentity,
    request: WritingWorkspaceRequest,
    files: Vec<ManuscriptFile>,
) -> Result<WritingWorkspaceSnapshot, Error> {
    let id = request.manuscript_id.to_string();
    if files.len() > MAX_FILES || request.buffers.len() > MAX_FILES {
        return Err(Error::Budget);
    }
    let mut buffers = BTreeMap::new();
    let mut total = 0u64;
    for buffer in request.buffers {
        research_workbench_native_sdk::resource_path::validate_relative_path(&buffer.path)
            .map_err(|_| Error::InvalidInput)?;
        if buffer.editor.session_id.is_nil()
            || buffer.editor.revision.is_nil()
            || buffers.contains_key(&buffer.path)
        {
            return Err(Error::InvalidInput);
        }
        total = total
            .checked_add(buffer.text.len() as u64)
            .ok_or(Error::Budget)?;
        if buffer.text.len() as u64 > MAX_FILE_BYTES || total > MAX_WORKSPACE_BYTES {
            return Err(Error::Budget);
        }
        buffers.insert(buffer.path.clone(), buffer);
    }
    let source =
        crate::scoped_text_files::legacy::manuscript_source_directory(manuscript_root, &id)
            .map_err(|_| Error::Unavailable)?;
    let directory = Directory::root(&source).map_err(|_| Error::Unavailable)?;
    let mut manifest = WorkspaceManifest {
        schema_version: 1,
        identity: scope,
        files: vec![],
    };
    let mut inputs = vec![];
    total = 0;
    for file in files {
        research_agent_core::workspace::validate_path(&file.relative_path)
            .map_err(|_| Error::InvalidInput)?;
        if file.size_bytes < 0 {
            return Err(Error::Conflict);
        }
        let expected_bytes = (file.size_bytes as u64).max(
            buffers
                .get(&file.relative_path)
                .map_or(0, |buffer| buffer.text.len() as u64),
        );
        if total.checked_add(expected_bytes).ok_or(Error::Budget)? > MAX_WORKSPACE_BYTES {
            return Err(Error::Budget);
        }
        let (disk, executable) =
            read_registered(&directory, &file.relative_path, file.size_bytes as u64)?;
        let disk_hash = crate::scoped_text_files::legacy::sha256(&disk);
        if !disk_hash.eq_ignore_ascii_case(&file.sha256) {
            return Err(Error::Conflict);
        }
        let (bytes, editor) = if let Some(buffer) = buffers.remove(&file.relative_path) {
            std::str::from_utf8(&disk).map_err(|_| Error::InvalidInput)?;
            if !disk_hash.eq_ignore_ascii_case(&buffer.expected_disk_sha256) {
                return Err(Error::Conflict);
            }
            (buffer.text.into_bytes(), Some(buffer.editor))
        } else {
            (disk, None)
        };
        total = total.checked_add(bytes.len() as u64).ok_or(Error::Budget)?;
        if total > MAX_WORKSPACE_BYTES {
            return Err(Error::Budget);
        }
        let version = FileVersion {
            sha256: crate::scoped_text_files::legacy::sha256(&bytes),
            byte_size: bytes.len() as u64,
            executable,
        };
        manifest.files.push(BaselineFile {
            file: WorkspaceFile {
                path: file.relative_path.clone(),
                version,
            },
            source: SourceVersion {
                resource_id: format!("{id}/{}", file.relative_path),
                revision: format!("sha256:{disk_hash}"),
                editor,
            },
        });
        inputs.push(WorkspaceInput {
            path: file.relative_path,
            bytes,
        });
    }
    if !buffers.is_empty() {
        return Err(Error::Conflict);
    }
    manifest.files.sort_by(|a, b| a.file.path.cmp(&b.file.path));
    manifest.validate().map_err(|_| Error::InvalidInput)?;
    Ok(WritingWorkspaceSnapshot { manifest, inputs })
}
fn read_registered(
    root: &Directory,
    path: &str,
    expected_size: u64,
) -> Result<(Vec<u8>, bool), Error> {
    if expected_size > MAX_FILE_BYTES {
        return Err(Error::Budget);
    }
    let relative = research_workbench_native_sdk::resource_path::validate_relative_path(path)
        .map_err(|_| Error::InvalidInput)?;
    let mut components = relative.components().peekable();
    let mut current = root.child(Path::new(".")).map_err(|_| Error::Unavailable)?;
    while let Some(Component::Normal(name)) = components.next() {
        if components.peek().is_some() {
            current = current
                .child(Path::new(name))
                .map_err(|_| Error::Unavailable)?;
        } else {
            let file = current
                .config_file(name.to_str().ok_or(Error::InvalidInput)?)
                .map_err(|_| Error::Unavailable)?
                .ok_or(Error::Unavailable)?;
            let metadata = file.metadata().map_err(|_| Error::Unavailable)?;
            if metadata.nlink() != 1 || metadata.len() != expected_size {
                return Err(Error::Conflict);
            }
            let mut bytes = vec![];
            file.take(expected_size + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Unavailable)?;
            if bytes.len() as u64 != expected_size {
                return Err(Error::Conflict);
            }
            return Ok((bytes, metadata.mode() & 0o111 != 0));
        }
    }
    Err(Error::InvalidInput)
}
