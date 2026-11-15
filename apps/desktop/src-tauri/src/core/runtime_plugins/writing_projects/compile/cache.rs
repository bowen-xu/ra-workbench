//! Original bounded cache retention and filesystem cleanup.
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::storage::AppStore;

use super::{repository, resources::Sql};
use crate::agent::writing_error::WritingError;

const MAX_RUNS_PER_MANUSCRIPT: usize = 8;
const MAX_CACHE_BYTES_PER_MANUSCRIPT: u64 = 1024 * 1024 * 1024;
const MAX_RUN_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_CACHE_ENTRIES: usize = 100_000;

#[derive(Debug)]
struct RunDirectory {
    id: String,
    path: PathBuf,
    modified: SystemTime,
    byte_size: u64,
}

pub(crate) fn cleanup_manuscript_runs(
    sql: &Sql,
    store: &AppStore,
    manuscript_id: &str,
    current_run_id: Option<&str>,
) -> Result<(), WritingError> {
    let latest_successful_id = store.connection()?.query_row(
        sql.get("project-compile-cache-protected.v1.sql"),
        [manuscript_id],
        |row| row.get::<_, Option<String>>(0),
    )?;
    let mut protected = HashSet::new();
    if let Some(run_id) = current_run_id {
        protected.insert(run_id.to_owned());
    }
    if let Some(run_id) = latest_successful_id {
        protected.insert(run_id);
    }
    if let Some(run) = repository::latest_pdf_run(sql, &*store.connection()?, manuscript_id)? {
        protected.insert(run.id);
    }
    cleanup_with_limits(
        &store.writing_cache_directory().join(manuscript_id),
        &protected,
        MAX_RUNS_PER_MANUSCRIPT,
        MAX_CACHE_BYTES_PER_MANUSCRIPT,
        MAX_RUN_AGE,
    )
}

fn cleanup_with_limits(
    root: &Path,
    protected: &HashSet<String>,
    max_runs: usize,
    max_bytes: u64,
    max_age: Duration,
) -> Result<(), WritingError> {
    if !root.exists() {
        return Ok(());
    }
    let now = SystemTime::now();
    let mut directories = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            fs::remove_file(entry.path())?;
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        directories.push(RunDirectory {
            id: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            byte_size: directory_size(&entry.path())?,
        });
    }
    directories.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.id.cmp(&left.id))
    });

    let mut kept_runs = directories
        .iter()
        .filter(|directory| protected.contains(&directory.id))
        .count();
    let mut kept_bytes = directories
        .iter()
        .filter(|directory| protected.contains(&directory.id))
        .fold(0u64, |total, directory| {
            total.saturating_add(directory.byte_size)
        });
    for directory in directories {
        let is_protected = protected.contains(&directory.id);
        if is_protected {
            continue;
        }
        let is_fresh = now
            .duration_since(directory.modified)
            .is_ok_and(|age| age <= max_age);
        let within_count = kept_runs < max_runs;
        let within_bytes = kept_bytes.saturating_add(directory.byte_size) <= max_bytes;
        if is_fresh && within_count && within_bytes {
            kept_runs += 1;
            kept_bytes = kept_bytes.saturating_add(directory.byte_size);
        } else {
            fs::remove_dir_all(directory.path)?;
        }
    }
    Ok(())
}

fn directory_size(root: &Path) -> Result<u64, WritingError> {
    let mut pending = vec![root.to_path_buf()];
    let mut byte_size = 0u64;
    let mut entries = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            entries += 1;
            if entries > MAX_CACHE_ENTRIES {
                return Err(WritingError::Process(
                    "LaTeX 构建缓存包含过多文件，已拒绝继续编译".to_owned(),
                ));
            }
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                byte_size = byte_size.saturating_add(metadata.len());
            } else if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                byte_size = byte_size.saturating_add(metadata.len());
            }
        }
    }
    Ok(byte_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn retains_protected_runs_while_enforcing_count_and_size() {
        let root = std::env::temp_dir().join(format!("writing-cache-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create cache root");
        for index in 0..10 {
            let run = root.join(format!("run-{index:02}"));
            fs::create_dir_all(&run).expect("create run cache");
            fs::write(run.join("artifact"), [0u8; 4]).expect("write cache artifact");
        }
        let protected = HashSet::from(["run-00".to_owned(), "run-01".to_owned()]);

        cleanup_with_limits(&root, &protected, 4, 12, Duration::from_secs(60))
            .expect("clean cache");

        let remaining = fs::read_dir(&root)
            .expect("read cache")
            .map(|entry| {
                entry
                    .expect("cache entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<HashSet<_>>();
        assert!(remaining.contains("run-00"));
        assert!(remaining.contains("run-01"));
        assert!(remaining.len() <= 4);
        assert!(directory_size(&root).expect("cache size") <= 16);
        fs::remove_dir_all(root).expect("remove cache fixture");
    }

    #[test]
    fn removes_symlinks_without_following_them() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let root = std::env::temp_dir().join(format!("writing-cache-{}", Uuid::new_v4()));
            let outside =
                std::env::temp_dir().join(format!("writing-cache-outside-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("create cache root");
            fs::create_dir_all(&outside).expect("create outside directory");
            fs::write(outside.join("keep"), "safe").expect("write outside file");
            symlink(&outside, root.join("unsafe-link")).expect("create cache symlink");

            cleanup_with_limits(&root, &HashSet::new(), 4, 1024, Duration::from_secs(60))
                .expect("clean cache symlink");

            assert!(!root.join("unsafe-link").exists());
            assert_eq!(
                fs::read_to_string(outside.join("keep")).expect("outside file"),
                "safe"
            );
            fs::remove_dir_all(root).expect("remove cache fixture");
            fs::remove_dir_all(outside).expect("remove outside fixture");
        }
    }
}
