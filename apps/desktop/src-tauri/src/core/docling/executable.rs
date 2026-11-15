//! One stable server path, published atomically without changing mapped files.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use fs2::FileExt;

use super::{error::DoclingError, launcher::create_private_directory};

pub(super) fn prepare_executable(
    source: &Path,
    service_root: &Path,
) -> Result<PathBuf, DoclingError> {
    let bytes = fs::read(source)?;
    let directory = service_root.join("bin");
    create_private_directory(&directory)?;
    // Model instances have separate startup locks, but share this executable.
    let publication_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("publish.lock"))?;
    FileExt::lock_exclusive(&publication_lock)?;
    let path = directory.join(
        source
            .file_name()
            .ok_or_else(|| DoclingError::Start("原生解析程序路径无效".into()))?,
    );
    if path.exists() && fs::read(&path)? == bytes {
        return Ok(path);
    }
    // Rename replaces the directory entry, not the file mapped by an existing
    // process. The path stays fixed and unchanged binaries keep their inode.
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    temporary.write_all(&bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_updates_keep_one_path_and_preserve_open_executables() {
        use std::io::Read;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("daemon");
        let service_root = root.path().join("service");
        fs::write(&source, b"first").unwrap();
        let first = prepare_executable(&source, &service_root).unwrap();
        let mut previous = fs::File::open(&first).unwrap();
        assert_eq!(first, service_root.join("bin/daemon"));
        assert_eq!(first, prepare_executable(&source, &service_root).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            assert_ne!(
                fs::metadata(&source).unwrap().ino(),
                fs::metadata(&first).unwrap().ino()
            );
            assert_eq!(
                fs::metadata(&first).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        fs::write(&source, b"second").unwrap();
        let second = prepare_executable(&source, &service_root).unwrap();
        assert_eq!(first, second);
        let mut previous_bytes = Vec::new();
        previous.read_to_end(&mut previous_bytes).unwrap();
        assert_eq!(previous_bytes, b"first");
        assert_eq!(fs::read(second).unwrap(), b"second");
        assert!(!service_root.join("executables").exists());
    }

    #[test]
    fn identical_server_is_untouched_and_damaged_copy_is_repaired() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("daemon");
        fs::write(&source, b"first").unwrap();
        let executable = prepare_executable(&source, root.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let inode = fs::metadata(&executable).unwrap().ino();
            prepare_executable(&source, root.path()).unwrap();
            assert_eq!(fs::metadata(&executable).unwrap().ino(), inode);
        }
        fs::write(&executable, b"damaged").unwrap();
        assert_eq!(
            prepare_executable(&source, root.path()).unwrap(),
            executable
        );
        assert_eq!(fs::read(executable).unwrap(), b"first");
    }

    #[test]
    fn model_instances_share_the_publication_lock() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("daemon");
        fs::write(&source, b"server").unwrap();
        let directory = root.path().join("bin");
        create_private_directory(&directory).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("publish.lock"))
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();
        let service_root = root.path().to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        let publishing = std::thread::spawn(move || {
            tx.send(prepare_executable(&source, &service_root)).unwrap();
        });
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        FileExt::unlock(&lock).unwrap();
        assert_eq!(rx.recv().unwrap().unwrap(), directory.join("daemon"));
        publishing.join().unwrap();
    }
}
