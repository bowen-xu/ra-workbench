//! Reject links and special files before reopening this CLI's synthetic store.
use super::{Error, Path, fs};

pub(super) fn validate_tree(directory: &Path) -> Result<(), Error> {
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err("structured source reopen requires an owned directory".into());
    }
    let mut pending = vec![directory.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err("structured source reopen rejects symlinks in its synthetic run".into());
        }
        if file_type.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        } else if !file_type.is_file() {
            return Err(
                "structured source reopen rejects special files in its synthetic run".into(),
            );
        }
    }
    Ok(())
}

#[path = "safety_tests.rs"]
#[cfg(all(test, unix))]
mod tests;
