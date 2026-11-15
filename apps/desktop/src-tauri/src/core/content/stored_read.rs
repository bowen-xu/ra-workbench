//! Internal object-read adapter. App retains the content guard while resolving
//! under its original database guard, then releases that database before read.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum StoredObjectReadError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("stored object file is missing")]
    MissingFile,
    #[error("unsafe stored object path")]
    UnsafePath,
}

/// An owned, resolved object location without a database or runtime owner.
/// This is internal native assembly, not an arbitrary-path plugin capability.
pub(crate) struct StoredObjectRead {
    canonical_path: PathBuf,
}

impl StoredObjectRead {
    /// Preserve the established path checks, including links that resolve to
    /// regular files inside the canonical root. The caller holds content_read.
    pub(crate) fn resolve(
        content_directory: &Path,
        relative_path: &str,
    ) -> Result<Self, StoredObjectReadError> {
        let relative_path = Path::new(relative_path);
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(StoredObjectReadError::UnsafePath);
        }

        let canonical_root = fs::canonicalize(content_directory)?;
        let stored_path = content_directory.join(relative_path);
        let canonical_path = match fs::canonicalize(stored_path) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(StoredObjectReadError::MissingFile);
            }
            Err(error) => return Err(error.into()),
        };
        if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
            return Err(StoredObjectReadError::UnsafePath);
        }
        Ok(Self { canonical_path })
    }

    /// App calls this after dropping its database guard, while content_read is
    /// still held. Late I/O errors remain I/O errors, including NotFound.
    pub(crate) fn read(self) -> io::Result<Vec<u8>> {
        fs::read(self.canonical_path)
    }

    /// Compatibility only for the existing internal attachment-path adapter.
    /// Ordinary plugin business APIs do not expose this path or the root.
    pub(crate) fn into_legacy_path(self) -> PathBuf {
        self.canonical_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_original_path_cases_and_reads_without_a_connection() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("objects");
        fs::create_dir_all(root.join("nested")).unwrap();
        let bytes = [0, 255, 42, 0, 17];
        let file = root.join("nested/object.bin");
        fs::write(&file, bytes).unwrap();

        // Neither the input strings nor a query borrow are retained.
        let read = {
            let root = root.clone();
            let relative = String::from("./nested/object.bin");
            StoredObjectRead::resolve(&root, &relative).unwrap()
        };
        assert_eq!(read.read().unwrap(), bytes);
        assert_eq!(
            StoredObjectRead::resolve(&root, "nested/object.bin")
                .unwrap()
                .into_legacy_path(),
            fs::canonicalize(&file).unwrap()
        );

        let absent_root = directory.path().join("absent");
        for path in ["../outside.bin", "nested/../../outside.bin"] {
            assert!(matches!(
                StoredObjectRead::resolve(&absent_root, path),
                Err(StoredObjectReadError::UnsafePath)
            ));
        }
        assert!(matches!(
            StoredObjectRead::resolve(&absent_root, file.to_str().unwrap()),
            Err(StoredObjectReadError::UnsafePath)
        ));
        assert!(matches!(
            StoredObjectRead::resolve(&absent_root, "object.bin"),
            Err(StoredObjectReadError::Io(error)) if error.kind() == io::ErrorKind::NotFound
        ));
        assert!(matches!(
            StoredObjectRead::resolve(&root, "absent.bin"),
            Err(StoredObjectReadError::MissingFile)
        ));
        assert!(matches!(
            StoredObjectRead::resolve(&root, "nested"),
            Err(StoredObjectReadError::UnsafePath)
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&file, root.join("internal-link")).unwrap();
            symlink(root.join("nested"), root.join("nested-link")).unwrap();
            let root_link = directory.path().join("root-link");
            symlink(&root, &root_link).unwrap();
            for (base, path) in [
                (&root, "internal-link"),
                (&root, "nested-link/object.bin"),
                (&root_link, "nested/object.bin"),
            ] {
                assert_eq!(
                    StoredObjectRead::resolve(base, path)
                        .unwrap()
                        .read()
                        .unwrap(),
                    bytes
                );
            }
            let outside = directory.path().join("outside.bin");
            fs::write(&outside, b"outside").unwrap();
            symlink(&outside, root.join("outside-link")).unwrap();
            assert!(matches!(
                StoredObjectRead::resolve(&root, "outside-link"),
                Err(StoredObjectReadError::UnsafePath)
            ));
        }

        let read = StoredObjectRead::resolve(&root, "nested/object.bin").unwrap();
        fs::remove_file(&file).unwrap();
        assert_eq!(read.read().unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
