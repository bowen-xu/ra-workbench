//! Original atomic replacement and cleanup sequence; no publication policy.
use super::TextFileError;
use std::{fs, io::Write, path::Path};
use uuid::Uuid;

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), TextFileError> {
    atomic_write_with(path, bytes, |temporary, content| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary)?;
        file.write_all(content)?;
        file.sync_all()
    })
}

pub(crate) fn atomic_write_with(
    path: &Path,
    bytes: &[u8],
    write_temporary: impl FnOnce(&Path, &[u8]) -> Result<(), std::io::Error>,
) -> Result<(), TextFileError> {
    let parent = path.parent().ok_or(TextFileError::InvalidPath)?;
    let temporary = parent.join(format!(".writing-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        write_temporary(&temporary, bytes)?;
        fs::rename(&temporary, path)?;
        sync_directory(parent)?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Into::into)
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

pub(crate) fn atomic_create(path: &Path, bytes: &[u8]) -> Result<(), TextFileError> {
    let parent = path.parent().ok_or(TextFileError::InvalidPath)?;
    let temporary = parent.join(format!(".writing-create-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::hard_link(&temporary, path)?;
        fs::remove_file(&temporary)?;
        sync_directory(parent)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return if error.kind() == std::io::ErrorKind::AlreadyExists {
            Err(TextFileError::FileConflict)
        } else {
            Err(error.into())
        };
    }
    Ok(())
}
