use std::path::{Path, PathBuf};

use raft_engine::{Config, ReadableSize};

/// Small desktop log settings. Purge is caller-driven and these thresholds are
/// maintenance triggers, not disk/memory limits or a retention policy.
#[derive(Clone, Debug)]
pub struct LogConfig {
    pub path: PathBuf,
    pub target_file_bytes: u64,
    pub purge_threshold_bytes: u64,
    pub rewrite_threshold_bytes: u64,
    /// Hard bound for this owner's managed file extents plus reservations.
    /// Filesystem metadata / volume-wide competing writers are not this quota.
    pub max_managed_file_bytes: u64,
    pub maintenance_reserve_bytes: u64,
}

impl LogConfig {
    pub fn desktop(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            target_file_bytes: 4 * 1024 * 1024,
            purge_threshold_bytes: 4 * 1024 * 1024,
            rewrite_threshold_bytes: 4 * 1024 * 1024,
            max_managed_file_bytes: 256 * 1024 * 1024,
            maintenance_reserve_bytes: 16 * 1024 * 1024,
        }
    }

    pub(crate) fn engine_config(&self) -> crate::LogResult<Config> {
        let dir = self
            .path
            .to_str()
            .ok_or(crate::LogError::InvalidArgument("log path is not UTF-8"))?;
        if self.target_file_bytes == 0
            || self.purge_threshold_bytes < self.target_file_bytes
            || self.rewrite_threshold_bytes < self.target_file_bytes
            || self.maintenance_reserve_bytes < self.target_file_bytes.saturating_mul(2)
            || self.max_managed_file_bytes
                <= self.maintenance_reserve_bytes.saturating_add(128 * 1024)
            || self.max_managed_file_bytes > (usize::MAX / 2) as u64
        {
            return Err(crate::LogError::InvalidArgument(
                "invalid file or purge threshold",
            ));
        }
        Ok(Config {
            dir: dir.to_owned(),
            target_file_size: ReadableSize(self.target_file_bytes),
            purge_threshold: ReadableSize(self.purge_threshold_bytes),
            purge_rewrite_threshold: Some(ReadableSize(self.rewrite_threshold_bytes)),
            recovery_threads: 1,
            enable_log_recycle: false,
            prefill_for_recycle: false,
            ..Default::default()
        })
    }
}
