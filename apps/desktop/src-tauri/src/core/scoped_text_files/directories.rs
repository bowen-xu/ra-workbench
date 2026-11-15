//! Original controlled directory algorithms and staged cleanup paths.
use super::{
    TextFileError,
    legacy::{manuscript_source_directory, reject_symlink_components, validate_relative_path},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;
pub(crate) fn list_directories(
    manuscript_directory: &Path,
    manuscript_id: &str,
) -> Result<Vec<String>, TextFileError> {
    let root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let mut directories = Vec::new();
    collect_directories(&root, &root, &mut directories)?;
    directories.sort();
    Ok(directories)
}

pub(crate) fn create_directory(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
) -> Result<(), TextFileError> {
    let root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let destination = root.join(validate_relative_path(relative_path)?);
    reject_symlink_components(&root, &destination)?;
    if destination.exists() {
        return Err(TextFileError::FileConflict);
    }
    fs::create_dir_all(destination)?;
    Ok(())
}

pub(crate) fn rename_directory(
    manuscript_directory: &Path,
    manuscript_id: &str,
    from_path: &str,
    to_path: &str,
) -> Result<(), TextFileError> {
    let root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let source = root.join(validate_relative_path(from_path)?);
    let destination = root.join(validate_relative_path(to_path)?);
    reject_symlink_components(&root, &source)?;
    reject_symlink_components(&root, &destination)?;
    if !source.is_dir() {
        return Err(TextFileError::FileNotFound);
    }
    if destination.exists() || destination.starts_with(&source) {
        return Err(TextFileError::FileConflict);
    }
    fs::create_dir_all(destination.parent().ok_or(TextFileError::InvalidPath)?)?;
    fs::rename(source, destination)?;
    Ok(())
}

pub(crate) fn stage_directory_removal(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
) -> Result<PathBuf, TextFileError> {
    let source_root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let source = source_root.join(validate_relative_path(relative_path)?);
    reject_symlink_components(&source_root, &source)?;
    if !source.is_dir() {
        return Err(TextFileError::FileNotFound);
    }
    let manuscript_root = source_root.parent().ok_or(TextFileError::InvalidPath)?;
    let staged = manuscript_root.join(format!(".writing-delete-{}", Uuid::new_v4()));
    fs::rename(source, &staged)?;
    Ok(staged)
}

pub(crate) fn restore_staged_directory(
    manuscript_directory: &Path,
    manuscript_id: &str,
    relative_path: &str,
    staged: &Path,
) -> Result<(), TextFileError> {
    let source_root = manuscript_source_directory(manuscript_directory, manuscript_id)?;
    let destination = source_root.join(validate_relative_path(relative_path)?);
    fs::create_dir_all(destination.parent().ok_or(TextFileError::InvalidPath)?)?;
    fs::rename(staged, destination)?;
    Ok(())
}

fn collect_directories(
    root: &Path,
    directory: &Path,
    output: &mut Vec<String>,
) -> Result<(), TextFileError> {
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
            return Err(TextFileError::SymlinkNotAllowed);
        }
        if !metadata.is_dir() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| TextFileError::InvalidPath)?
            .to_str()
            .ok_or(TextFileError::InvalidPath)?
            .replace('\\', "/");
        validate_relative_path(&relative)?;
        output.push(relative);
        collect_directories(root, &entry.path(), output)?;
    }
    Ok(())
}

pub(crate) fn validate_directory(root: &Path, path: &Path) -> Result<(), TextFileError> {
    for candidate in [root, path] {
        match fs::symlink_metadata(candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(TextFileError::SymlinkNotAllowed);
            }
            Ok(metadata) if !metadata.is_dir() => return Err(TextFileError::InvalidPath),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn remove_directory(path: &Path) -> Result<(), TextFileError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
