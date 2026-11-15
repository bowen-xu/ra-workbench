//! Original bounded resource-byte staging and filesystem publication.
use crate::agent::writing_error::WritingError;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub(crate) const MAX_RESOURCE_FILES: usize = 256;
const MAX_RESOURCE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) struct StagedResource {
    pub(crate) relative_path: String,
    pub(crate) staged_path: PathBuf,
    pub(crate) byte_size: u64,
    pub(crate) sha256: String,
    pub(crate) original_bibtex: Option<String>,
}

pub(crate) fn create_note_asset(
    source_root: &Path,
    relative_path: &str,
    bytes: &[u8],
) -> Result<PathBuf, WritingError> {
    let destination = source_root.join(relative_path);
    crate::scoped_text_files::legacy::reject_symlink_components(source_root, &destination)?;
    fs::create_dir_all(destination.parent().ok_or(WritingError::InvalidPath)?)?;
    crate::scoped_text_files::legacy::reject_symlink_components(source_root, &destination)?;
    crate::scoped_text_files::legacy::atomic_create(&destination, bytes)?;
    Ok(destination)
}

pub(crate) fn read_note_asset(
    manuscript_directory: &Path,
    manuscript_id: &str,
    asset_id: &str,
) -> Result<Vec<u8>, WritingError> {
    let relative = research_workbench_native_sdk::resource_path::writing_note_asset_path(asset_id)
        .map_err(|_| WritingError::InvalidPath)?;
    let path = crate::scoped_text_files::legacy::resolve_existing_file(
        manuscript_directory,
        manuscript_id,
        &relative,
    )?;
    if fs::metadata(&path)?.len()
        > research_workbench_native_sdk::writing_note_asset::MAX_BYTES as u64
    {
        return Err(WritingError::FileTooLarge);
    }
    Ok(fs::read(path)?)
}

pub(crate) fn stage_resource(
    value: &str,
    source_root: &Path,
    destination_directory: &Path,
    staging_root: &Path,
    seen: &mut HashSet<String>,
    total_bytes: &mut u64,
) -> Result<StagedResource, WritingError> {
    let source = validated_source_file(value)?;
    let filename = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(WritingError::InvalidPath)?;
    if !seen.insert(filename.to_owned()) {
        return Err(WritingError::FileConflict);
    }
    let relative_path = destination_directory
        .join(filename)
        .to_str()
        .ok_or(WritingError::InvalidPath)?
        .replace('\\', "/");
    research_workbench_native_sdk::resource_path::validate_relative_path(&relative_path)
        .map_err(|_| WritingError::InvalidPath)?;
    if source_root.join(&relative_path).exists() {
        return Err(WritingError::FileConflict);
    }
    let staged_path = staging_root.join(filename);
    let (byte_size, sha256) = copy_and_hash(&source, &staged_path)?;
    *total_bytes = total_bytes.saturating_add(byte_size);
    if *total_bytes > MAX_RESOURCE_BYTES {
        return Err(WritingError::FileTooLarge);
    }
    Ok(StagedResource {
        relative_path,
        staged_path,
        byte_size,
        sha256,
        original_bibtex: None,
    })
}
pub(crate) fn publish_files(
    source_root: &Path,
    resources: &[StagedResource],
) -> Result<Vec<PathBuf>, WritingError> {
    let mut published = Vec::new();
    let filesystem_result = (|| {
        for resource in resources {
            let destination = source_root.join(&resource.relative_path);
            fs::create_dir_all(destination.parent().ok_or(WritingError::InvalidPath)?)?;
            fs::rename(&resource.staged_path, &destination)?;
            published.push(destination);
        }
        Ok::<_, WritingError>(())
    })();
    if let Err(error) = filesystem_result {
        remove_published(&published);
        return Err(error);
    }

    Ok(published)
}
fn validated_source_file(value: &str) -> Result<PathBuf, WritingError> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        return Err(WritingError::InvalidPath);
    }
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(WritingError::SymlinkNotAllowed);
    }
    if !metadata.is_file() {
        return Err(WritingError::InvalidPath);
    }
    Ok(path.canonicalize()?)
}

fn copy_and_hash(source: &Path, destination: &Path) -> Result<(u64, String), WritingError> {
    let mut input = File::open(source)?;
    let expected_size = input.metadata()?.len();
    if expected_size > MAX_RESOURCE_BYTES {
        return Err(WritingError::FileTooLarge);
    }
    let mut output = File::options()
        .create_new(true)
        .write(true)
        .open(destination)?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied = copied.saturating_add(count as u64);
        if copied > expected_size || copied > MAX_RESOURCE_BYTES {
            return Err(WritingError::FileTooLarge);
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }
    if copied != expected_size {
        return Err(WritingError::FileConflict);
    }
    output.sync_all()?;
    Ok((copied, format!("{:x}", hasher.finalize())))
}

pub(crate) fn remove_published(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}
