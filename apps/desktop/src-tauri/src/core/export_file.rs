//! Publish complete external exports without overwriting concurrent downloads.
use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};
use uuid::Uuid;

pub(crate) fn publish_external_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    // Sibling paths share a filesystem. Linking atomically refuses an existing
    // destination, including a dangling symlink, and publishes a synced inode.
    fs::hard_link(temporary, destination)?;
    fs::remove_file(temporary)
}

pub(crate) fn write_new_file(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = destination.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let temporary = parent.join(format!(".research-export-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = File::options()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        publish_external_file(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writes_complete_bytes_without_overwriting_or_leaving_staging_files() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("export.json");
        write_new_file(&target, b"first").unwrap();
        assert_eq!(
            write_new_file(&target, b"second").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&target).unwrap(), b"first");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        assert!(write_new_file(&root.path().join("absent/file.json"), b"data").is_err());
    }

    #[test]
    fn concurrent_publishers_have_one_complete_winner() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("race.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let results = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for byte in 1..=4_u8 {
                let target = destination.clone();
                let barrier = barrier.clone();
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    (byte, write_new_file(&target, &vec![byte; 4096]))
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let winners: Vec<_> = results
            .iter()
            .filter(|(_, result)| result.is_ok())
            .collect();
        assert_eq!(winners.len(), 1);
        for (_, result) in &results {
            if let Err(error) = result {
                assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            }
        }
        assert_eq!(fs::read(destination).unwrap(), vec![winners[0].0; 4096]);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_destination_link_is_a_collision_not_a_write_target() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("download.json");
        let absent = root.path().join("absent.json");
        std::os::unix::fs::symlink(&absent, &destination).unwrap();
        assert_eq!(
            write_new_file(&destination, b"data").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(!absent.exists());
        assert_eq!(fs::read_link(destination).unwrap(), absent);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
