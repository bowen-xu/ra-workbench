//! Internal compatibility for existing native file adapters. Bibliography policy
//! receives ScopedTextFiles and cannot obtain these paths from that capability.
use super::TextFileError;
pub(crate) use super::directories::{
    create_directory, list_directories, rename_directory, restore_staged_directory,
    stage_directory_removal,
};
pub(crate) use super::{
    atomic::{atomic_create, atomic_write},
    path::validate_relative_path,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_TEXT_FILE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn read_text_file(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
) -> Result<(String, String), TextFileError> {
    let path = resolve_existing_file(manuscript_directory, manuscript_id, relative_path)?;
    let metadata = fs::metadata(&path)?;
    if metadata.len() > MAX_TEXT_FILE_BYTES as u64 {
        return Err(TextFileError::FileTooLarge);
    }
    let bytes = fs::read(path)?;
    let content = String::from_utf8(bytes).map_err(|_| TextFileError::UnsupportedEncoding)?;
    let hash = sha256(content.as_bytes());
    Ok((content, hash))
}

pub(crate) fn save_text_file(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
    content: &str,
    expected_sha256: Option<&str>,
) -> Result<String, TextFileError> {
    if content.len() > MAX_TEXT_FILE_BYTES {
        return Err(TextFileError::FileTooLarge);
    }
    let root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    reject_symlink_components(&root, &root)?;
    let relative = validate_relative_path(relative_path)?;
    let destination = root.join(relative);
    if destination.exists() {
        reject_symlink_components(&root, &destination)?;
        if let Some(expected) = expected_sha256 {
            let current = sha256(&fs::read(&destination)?);
            if current != expected {
                return Err(TextFileError::FileConflict);
            }
        }
    }
    fs::create_dir_all(destination.parent().ok_or(TextFileError::InvalidPath)?)?;
    reject_symlink_components(
        &root,
        destination.parent().ok_or(TextFileError::InvalidPath)?,
    )?;
    atomic_write(&destination, content.as_bytes())?;
    Ok(sha256(content.as_bytes()))
}

pub(crate) fn create_text_file_with_content(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
    content: &str,
) -> Result<String, TextFileError> {
    if content.len() > MAX_TEXT_FILE_BYTES {
        return Err(TextFileError::FileTooLarge);
    }
    let root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let destination = root.join(validate_relative_path(relative_path)?);
    reject_symlink_components(&root, &destination)?;
    if destination.exists() {
        return Err(TextFileError::FileConflict);
    }
    fs::create_dir_all(destination.parent().ok_or(TextFileError::InvalidPath)?)?;
    reject_symlink_components(
        &root,
        destination.parent().ok_or(TextFileError::InvalidPath)?,
    )?;
    atomic_create(&destination, content.as_bytes())?;
    Ok(sha256(content.as_bytes()))
}

pub(crate) fn rename_file(
    manuscript_directory: &Path,
    manuscript_id: &str,
    from_path: &str,
    to_path: &str,
    touch_destination: bool,
) -> Result<(), TextFileError> {
    let root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let source = root.join(validate_relative_path(from_path)?);
    let destination = root.join(validate_relative_path(to_path)?);
    reject_symlink_components(&root, &source)?;
    reject_symlink_components(&root, &destination)?;
    if !source.is_file() {
        return Err(TextFileError::FileNotFound);
    }
    if destination.exists() {
        return Err(TextFileError::FileConflict);
    }
    fs::create_dir_all(destination.parent().ok_or(TextFileError::InvalidPath)?)?;
    fs::rename(source, &destination)?;
    if touch_destination {
        fs::File::options()
            .write(true)
            .open(&destination)?
            .set_modified(std::time::SystemTime::now())?;
    }
    Ok(())
}

pub(crate) fn delete_file(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
) -> Result<(), TextFileError> {
    let path = resolve_existing_file(manuscript_directory, manuscript_id, relative_path)?;
    fs::remove_file(path)?;
    Ok(())
}

pub(crate) fn resolve_existing_file(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
) -> Result<PathBuf, TextFileError> {
    let root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let path = root.join(validate_relative_path(relative_path)?);
    reject_symlink_components(&root, &path)?;
    if !path.is_file() {
        return Err(TextFileError::FileNotFound);
    }
    Ok(path)
}

pub(crate) fn manuscript_source_directory(
    directory: &Path,
    manuscript_id: &str,
) -> Result<PathBuf, TextFileError> {
    if Uuid::parse_str(manuscript_id).is_err() {
        return Err(TextFileError::InvalidManuscriptId);
    }
    Ok(directory.join(manuscript_id).join("source"))
}

pub(crate) fn reject_symlink_components(root: &Path, path: &Path) -> Result<(), TextFileError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| TextFileError::InvalidPath)?;
    let mut current = root.to_path_buf();
    if let Ok(metadata) = fs::symlink_metadata(&current)
        && metadata.file_type().is_symlink()
    {
        return Err(TextFileError::SymlinkNotAllowed);
    }
    for component in relative.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            return Err(TextFileError::SymlinkNotAllowed);
        }
    }
    Ok(())
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) use super::directories::{remove_directory, validate_directory};
