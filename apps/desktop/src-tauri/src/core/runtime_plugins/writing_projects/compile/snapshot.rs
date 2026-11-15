//! Original ordered source snapshot and streaming manifest hash.
use crate::agent::writing_error::WritingError;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAX_SNAPSHOT_FILES: usize = 10_000;
const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) fn snapshot_source(source: &Path, destination: &Path) -> Result<String, WritingError> {
    let files = source_entries(source)?;
    let mut hasher = Sha256::new();
    for (relative, source_path, _) in files {
        let destination_path = destination.join(&relative);
        fs::create_dir_all(destination_path.parent().ok_or(WritingError::InvalidPath)?)?;
        let mut source_file = File::open(source_path)?;
        let mut destination_file = File::create(destination_path)?;
        hash_file(
            &relative,
            &mut source_file,
            Some(&mut destination_file),
            &mut hasher,
        )?;
        destination_file.sync_all()?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn source_manifest_hash(source: &Path) -> Result<String, WritingError> {
    let files = source_entries(source)?;
    let mut hasher = Sha256::new();
    for (relative, source_path, _) in files {
        let mut source_file = File::open(source_path)?;
        hash_file(&relative, &mut source_file, None, &mut hasher)?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn source_entries(source: &Path) -> Result<Vec<(PathBuf, PathBuf, u64)>, WritingError> {
    let mut files = Vec::new();
    collect_files(source, source, &mut files)?;
    files.retain(|(path, _, _)| !research_workbench_native_sdk::resource_path::is_note_input(path));
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.len() > MAX_SNAPSHOT_FILES {
        return Err(WritingError::Process(
            "稿件文件数量超过 10000 个".to_owned(),
        ));
    }
    let total_bytes = files.iter().map(|(_, _, size)| *size).sum::<u64>();
    if total_bytes > MAX_SNAPSHOT_BYTES {
        return Err(WritingError::Process("稿件工程超过 512 MiB".to_owned()));
    }
    Ok(files)
}

fn hash_file(
    relative: &Path,
    source: &mut File,
    mut destination: Option<&mut File>,
    hasher: &mut Sha256,
) -> Result<(), WritingError> {
    hasher.update(relative.to_string_lossy().as_bytes());
    hasher.update([0]);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if let Some(destination) = destination.as_deref_mut() {
            destination.write_all(&buffer[..count])?;
        }
        hasher.update(&buffer[..count]);
    }
    hasher.update([0]);
    Ok(())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(PathBuf, PathBuf, u64)>,
) -> Result<(), WritingError> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if research_agent_core::workspace::is_recovery_file_name(
            &entry.file_name().to_string_lossy(),
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            return Err(WritingError::SymlinkNotAllowed);
        }
        if metadata.is_dir() {
            collect_files(root, &entry.path(), output)?;
        } else if metadata.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| WritingError::InvalidPath)?
                .to_path_buf();
            output.push((relative, entry.path(), metadata.len()));
        }
    }
    Ok(())
}
