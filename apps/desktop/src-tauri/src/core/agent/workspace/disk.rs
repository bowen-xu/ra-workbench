//! Blocking file primitives for the application coordinator. No model/IPC access.
//! Reuses workspace::atomic_create's staged hard-link publication, with anchored
//! directory handles and explicit uncertainty after publication.
use crate::agent::instructions::directory::Directory;
use nix::{
    fcntl::{AtFlags, OFlag, openat},
    sys::stat::{Mode, fchmod},
    unistd::{UnlinkatFlags, fsync, linkat, unlinkat},
};
use research_agent_core::workspace::{FileVersion, MAX_FILE_BYTES, validate_path};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiskError {
    #[error("exclusive file renaming is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("invalid proposal file")]
    Invalid,
    #[error("proposal destination changed or is occupied")]
    Conflict,
    #[error("proposal file storage unavailable")]
    Unavailable,
    #[error("proposal file publication requires reconciliation")]
    RecoveryRequired,
}

pub struct DiskTarget {
    directory: Directory,
    parent: PathBuf,
    name: String,
    relative: String,
}

impl DiskTarget {
    /// Root is an authorized manuscript source directory, never model supplied.
    /// Parents must already exist; directory creation has a separate lifecycle.
    pub fn open(root: &Path, relative: &str) -> Result<Self, DiskError> {
        if !root.is_absolute() {
            return Err(DiskError::Invalid);
        }
        validate_path(relative).map_err(|_| DiskError::Invalid)?;
        let mut directory = Directory::root(root).map_err(|_| DiskError::Conflict)?;
        let device = directory.identity().map_err(|_| DiskError::Unavailable)?.0;
        let mut parent = root.to_path_buf();
        let parts: Vec<_> = relative.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            directory = directory
                .child(Path::new(part))
                .map_err(|_| DiskError::Conflict)?;
            if directory.identity().map_err(|_| DiskError::Unavailable)?.0 != device {
                return Err(DiskError::Conflict);
            }
            parent.push(part);
        }
        Ok(Self {
            directory,
            parent,
            name: parts[parts.len() - 1].into(),
            relative: relative.into(),
        })
    }

    fn check_parent(&self) -> Result<(), DiskError> {
        let current = Directory::root(&self.parent).map_err(|_| DiskError::Conflict)?;
        if current.identity().map_err(|_| DiskError::Unavailable)?
            != self
                .directory
                .identity()
                .map_err(|_| DiskError::Unavailable)?
        {
            return Err(DiskError::Conflict);
        }
        Ok(())
    }

    /// Commit dispatch intent first. Retrying an existing destination is a
    /// conflict even when its bytes match: only the coordinator can reconcile it.
    pub fn create(
        &self,
        operation: Uuid,
        bytes: &[u8],
        expected: &FileVersion,
    ) -> Result<FileVersion, DiskError> {
        if operation.is_nil()
            || bytes.len() as u64 > MAX_FILE_BYTES
            || expected.byte_size != bytes.len() as u64
            || expected.sha256 != format!("{:x}", Sha256::digest(bytes))
        {
            return Err(DiskError::Invalid);
        }
        self.check_parent()?;
        let temporary = self.temporary_name(operation);
        let mut file = File::from(
            openat(
                &self.directory,
                temporary.as_str(),
                OFlag::O_RDWR
                    | OFlag::O_CREAT
                    | OFlag::O_EXCL
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_CLOEXEC,
                Mode::S_IRUSR | Mode::S_IWUSR,
            )
            .map_err(|error| {
                if error == nix::errno::Errno::EEXIST {
                    DiskError::RecoveryRequired
                } else {
                    DiskError::Unavailable
                }
            })?,
        );
        let prepared = (|| {
            file.write_all(bytes).map_err(|_| DiskError::Unavailable)?;
            fchmod(
                &file,
                if expected.executable {
                    Mode::S_IRWXU
                } else {
                    Mode::S_IRUSR | Mode::S_IWUSR
                },
            )
            .map_err(|_| DiskError::Unavailable)?;
            file.sync_all().map_err(|_| DiskError::Unavailable)?;
            self.check_parent()?;
            linkat(
                &self.directory,
                temporary.as_str(),
                &self.directory,
                self.name.as_str(),
                AtFlags::empty(),
            )
            .map_err(|error| {
                if error == nix::errno::Errno::EEXIST {
                    DiskError::Conflict
                } else {
                    DiskError::Unavailable
                }
            })
        })();
        if let Err(error) = prepared {
            // Only this call's exclusively created staging entry is cleaned.
            unlinkat(
                &self.directory,
                temporary.as_str(),
                UnlinkatFlags::NoRemoveDir,
            )
            .map_err(|_| DiskError::RecoveryRequired)?;
            fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
            return Err(error);
        }
        // From here publication may have happened. Never delete the destination
        // or claim a clean failure if sync, cleanup or verification fails.
        fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
        unlinkat(
            &self.directory,
            temporary.as_str(),
            UnlinkatFlags::NoRemoveDir,
        )
        .map_err(|_| DiskError::RecoveryRequired)?;
        fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
        self.check_parent()
            .map_err(|_| DiskError::RecoveryRequired)?;
        let actual = self
            .observe()
            .map_err(|_| DiskError::RecoveryRequired)?
            .ok_or(DiskError::RecoveryRequired)?;
        if &actual != expected {
            return Err(DiskError::RecoveryRequired);
        }
        Ok(actual)
    }

    /// Read actual bounded bytes for reconciliation; missing is distinct from
    /// unreadable, symlink, hard-link or special-file destinations.
    pub fn observe(&self) -> Result<Option<FileVersion>, DiskError> {
        self.observe_with_links(1)
    }

    fn observe_with_links(&self, links: u64) -> Result<Option<FileVersion>, DiskError> {
        self.check_parent()?;
        let Some(mut file) = self
            .directory
            .config_file(&self.name)
            .map_err(|_| DiskError::Conflict)?
        else {
            return Ok(None);
        };
        let before = file.metadata().map_err(|_| DiskError::Unavailable)?;
        if !before.is_file() || before.nlink() != links || before.len() > MAX_FILE_BYTES {
            return Err(DiskError::Conflict);
        }
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|_| DiskError::Unavailable)?;
            if count == 0 {
                break;
            }
            size += count as u64;
            if size > MAX_FILE_BYTES {
                return Err(DiskError::Conflict);
            }
            hash.update(&buffer[..count]);
        }
        let after = file.metadata().map_err(|_| DiskError::Unavailable)?;
        let current = self
            .directory
            .config_file(&self.name)
            .map_err(|_| DiskError::Conflict)?
            .ok_or(DiskError::Conflict)?
            .metadata()
            .map_err(|_| DiskError::Unavailable)?;
        if before.len() != size
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.ino() != current.ino()
            || before.dev() != current.dev()
            || current.nlink() != links
            || after.len() != current.len()
            || after.ctime() != current.ctime()
            || after.ctime_nsec() != current.ctime_nsec()
        {
            return Err(DiskError::Conflict);
        }
        self.check_parent()?;
        Ok(Some(FileVersion {
            sha256: format!("{:x}", hash.finalize()),
            byte_size: size,
            executable: after.mode() & 0o111 != 0,
        }))
    }

    fn temporary_name(&self, operation: Uuid) -> String {
        format!(
            ".agent-create-{operation}-{:x}.tmp",
            Sha256::digest(self.relative.as_bytes())
        )
    }

    /// Caller verified a durable dispatched create and owns native write
    /// coordination. Only remove this operation's redundant link after checking
    /// identity and frozen content; never publish an unlinked stage or remove the
    /// destination. Unrelated, modified and incomplete artifacts stay for review.
    pub fn reconcile_create(
        &self,
        operation: Uuid,
        expected: &FileVersion,
    ) -> Result<FileVersion, DiskError> {
        if operation.is_nil() || expected.byte_size > MAX_FILE_BYTES {
            return Err(DiskError::Invalid);
        }
        self.check_parent()?;
        let temporary = self.temporary_name(operation);
        let Some(stage) = self
            .directory
            .config_file(&temporary)
            .map_err(|_| DiskError::Conflict)?
        else {
            return self
                .observe()?
                .filter(|actual| actual == expected)
                .ok_or(DiskError::RecoveryRequired);
        };
        let staged = stage.metadata().map_err(|_| DiskError::Unavailable)?;
        let destination = self
            .directory
            .config_file(&self.name)
            .map_err(|_| DiskError::Conflict)?
            .ok_or(DiskError::RecoveryRequired)?;
        let target = destination.metadata().map_err(|_| DiskError::Unavailable)?;
        if staged.nlink() != 2
            || target.nlink() != 2
            || staged.ino() != target.ino()
            || staged.dev() != target.dev()
        {
            return Err(DiskError::Conflict);
        }
        if self.observe_with_links(2)?.as_ref() != Some(expected) {
            return Err(DiskError::Conflict);
        }
        stage.sync_all().map_err(|_| DiskError::RecoveryRequired)?;
        fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
        self.check_parent()?;
        let current = self
            .directory
            .config_file(&temporary)
            .map_err(|_| DiskError::Conflict)?
            .ok_or(DiskError::Conflict)?
            .metadata()
            .map_err(|_| DiskError::Unavailable)?;
        let current_target = self
            .directory
            .config_file(&self.name)
            .map_err(|_| DiskError::Conflict)?
            .ok_or(DiskError::Conflict)?
            .metadata()
            .map_err(|_| DiskError::Unavailable)?;
        if current.ino() != staged.ino()
            || current.dev() != staged.dev()
            || current.nlink() != 2
            || current.ctime() != staged.ctime()
            || current.ctime_nsec() != staged.ctime_nsec()
            || current_target.ino() != current.ino()
            || current_target.dev() != current.dev()
        {
            return Err(DiskError::Conflict);
        }
        unlinkat(
            &self.directory,
            temporary.as_str(),
            UnlinkatFlags::NoRemoveDir,
        )
        .map_err(|_| DiskError::RecoveryRequired)?;
        fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
        self.observe()
            .map_err(|_| DiskError::RecoveryRequired)?
            .filter(|actual| actual == expected)
            .ok_or(DiskError::RecoveryRequired)
    }
}

#[cfg(test)]
#[path = "disk/tests.rs"]
mod tests;

#[path = "disk/mutate.rs"]
mod mutate;
