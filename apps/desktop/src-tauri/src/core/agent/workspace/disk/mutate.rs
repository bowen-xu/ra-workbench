//! Preserve originals before replacing/removing their directory entry. Backups
//! stay until the application coordinator has durably resolved the operation.
use super::*;

impl DiskTarget {
    fn backup(&self, operation: Uuid) -> Result<Self, DiskError> {
        if operation.is_nil() {
            return Err(DiskError::Invalid);
        }
        Ok(Self {
            directory: self
                .directory
                .child(Path::new("."))
                .map_err(|_| DiskError::Unavailable)?,
            parent: self.parent.clone(),
            name: format!(
                ".agent-before-{operation}-{:x}.bak",
                Sha256::digest(self.relative.as_bytes())
            ),
            relative: self.relative.clone(),
        })
    }

    fn preserve_existing(&self, operation: Uuid, before: &FileVersion) -> Result<(), DiskError> {
        let backup = self.backup(operation)?;
        if self.observe()?.as_ref() != Some(before) {
            return Err(DiskError::Conflict);
        }
        self.check_parent()?;
        rename_exclusive(&self.directory, &self.name, &backup.name)?;
        // After rename, the original is never silently removed, even if a source
        // race was captured or syncing failed. Recovery owns the preserved entry.
        backup
            .directory
            .config_file(&backup.name)
            .map_err(|_| DiskError::RecoveryRequired)?
            .ok_or(DiskError::RecoveryRequired)?
            .sync_all()
            .map_err(|_| DiskError::RecoveryRequired)?;
        fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
        if backup
            .observe()
            .map_err(|_| DiskError::RecoveryRequired)?
            .as_ref()
            != Some(before)
        {
            return Err(DiskError::RecoveryRequired);
        }
        Ok(())
    }

    /// Caller commits dispatch first and serializes manuscript writers. There is
    /// a recoverable interval with no destination; readers must respect that
    /// coordinator. A failure retains the backup and any competing destination.
    pub fn replace(
        &self,
        operation: Uuid,
        before: &FileVersion,
        bytes: &[u8],
        after: &FileVersion,
    ) -> Result<FileVersion, DiskError> {
        if bytes.len() as u64 > MAX_FILE_BYTES
            || after.byte_size != bytes.len() as u64
            || after.sha256 != format!("{:x}", Sha256::digest(bytes))
        {
            return Err(DiskError::Invalid);
        }
        self.preserve_existing(operation, before)?;
        self.create(operation, bytes, after)
            .map_err(|_| DiskError::RecoveryRequired)
    }

    /// Logical removal retains original bytes at the operation backup, not an
    /// unlink. Later cleanup requires durable application/undo resolution.
    pub fn remove(&self, operation: Uuid, before: &FileVersion) -> Result<(), DiskError> {
        self.preserve_existing(operation, before)?;
        if self
            .observe()
            .map_err(|_| DiskError::RecoveryRequired)?
            .is_some()
        {
            return Err(DiskError::RecoveryRequired);
        }
        Ok(())
    }

    /// Restore only a verified backup into an absent target; never overwrite a
    /// user's new file. This primitive does not decide whether rollback is due.
    pub fn restore_preserved(
        &self,
        operation: Uuid,
        before: &FileVersion,
    ) -> Result<FileVersion, DiskError> {
        let backup = self.backup(operation)?;
        if backup.observe()?.as_ref() != Some(before) {
            return Err(DiskError::Conflict);
        }
        self.check_parent()?;
        rename_exclusive(&self.directory, &backup.name, &self.name)?;
        self.directory
            .config_file(&self.name)
            .map_err(|_| DiskError::RecoveryRequired)?
            .ok_or(DiskError::RecoveryRequired)?
            .sync_all()
            .map_err(|_| DiskError::RecoveryRequired)?;
        fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
        self.observe()
            .map_err(|_| DiskError::RecoveryRequired)?
            .filter(|actual| actual == before)
            .ok_or(DiskError::RecoveryRequired)
    }

    pub fn observe_preserved(&self, operation: Uuid) -> Result<Option<FileVersion>, DiskError> {
        self.backup(operation)?.observe()
    }

    /// Preserve the replacement separately before restoring the original. Caller
    /// has committed rollback/recovery intent and owns manuscript coordination.
    pub fn rollback_replaced(
        &self,
        operation: Uuid,
        before: &FileVersion,
        after: &FileVersion,
    ) -> Result<FileVersion, DiskError> {
        let mut displaced = self.backup(operation)?;
        displaced.name = format!(
            ".agent-create-{operation}-{:x}.rollback",
            Sha256::digest(self.relative.as_bytes())
        );
        let original = self.observe_preserved(operation)?;
        let saved_after = displaced.observe()?;
        if saved_after.as_ref().is_some_and(|actual| actual != after) {
            return Err(DiskError::RecoveryRequired);
        }
        if original.is_none() {
            // Restoration may already have finished, or replacement never
            // removed the original. Confirm, never manufacture original bytes.
            return self.restore_removed(operation, before);
        }
        self.reconcile_preserved(operation, before)?;
        if self.observe()?.is_some() {
            if saved_after.is_some() {
                return Err(DiskError::RecoveryRequired);
            }
            self.reconcile_create(operation, after)?;
            self.check_parent()?;
            rename_exclusive(&self.directory, &self.name, &displaced.name)?;
        }
        // A changed source raced into the displaced name? Keep it and the
        // original backup; never overwrite it or silently report restoration.
        if let Some(actual) = displaced.observe()? {
            if actual != *after {
                return Err(DiskError::RecoveryRequired);
            }
            displaced
                .directory
                .config_file(&displaced.name)
                .map_err(|_| DiskError::RecoveryRequired)?
                .ok_or(DiskError::RecoveryRequired)?
                .sync_all()
                .map_err(|_| DiskError::RecoveryRequired)?;
            fsync(&displaced.directory).map_err(|_| DiskError::RecoveryRequired)?;
            if displaced.observe()?.as_ref() != Some(after) {
                return Err(DiskError::RecoveryRequired);
            }
        }
        // No displaced item is valid when replacement stopped before publish.
        self.restore_removed(operation, before)
    }

    /// Roll back a published addition by preserving it, never unlinking its
    /// bytes. An unobserved/never-published creation needs separate staging recovery.
    pub fn rollback_added(&self, operation: Uuid, after: &FileVersion) -> Result<(), DiskError> {
        if self.observe_preserved(operation)?.is_some() {
            self.reconcile_preserved(operation, after)?;
            if self.observe()?.is_some() {
                return Err(DiskError::RecoveryRequired);
            }
            return Ok(());
        }
        self.reconcile_create(operation, after)?;
        self.remove(operation, after)
    }

    /// Restore a removed file, or confirm an interrupted restoration. An
    /// occupied target plus a backup is ambiguous and never overwritten.
    pub fn restore_removed(
        &self,
        operation: Uuid,
        before: &FileVersion,
    ) -> Result<FileVersion, DiskError> {
        match (self.observe_preserved(operation)?, self.observe()?) {
            (Some(saved), None) if saved == *before => self.restore_preserved(operation, before),
            (None, Some(actual)) if actual == *before => {
                self.directory
                    .config_file(&self.name)
                    .map_err(|_| DiskError::RecoveryRequired)?
                    .ok_or(DiskError::RecoveryRequired)?
                    .sync_all()
                    .map_err(|_| DiskError::RecoveryRequired)?;
                fsync(&self.directory).map_err(|_| DiskError::RecoveryRequired)?;
                self.observe()
                    .map_err(|_| DiskError::RecoveryRequired)?
                    .filter(|actual| actual == before)
                    .ok_or(DiskError::RecoveryRequired)
            }
            _ => Err(DiskError::RecoveryRequired),
        }
    }

    /// Re-establish backup durability after an uncertain rename/sync boundary.
    /// Never restore, remove or substitute the preserved directory entry.
    pub fn reconcile_preserved(
        &self,
        operation: Uuid,
        expected: &FileVersion,
    ) -> Result<(), DiskError> {
        let backup = self.backup(operation)?;
        if backup.observe()?.as_ref() != Some(expected) {
            return Err(DiskError::RecoveryRequired);
        }
        backup
            .directory
            .config_file(&backup.name)
            .map_err(|_| DiskError::RecoveryRequired)?
            .ok_or(DiskError::RecoveryRequired)?
            .sync_all()
            .map_err(|_| DiskError::RecoveryRequired)?;
        fsync(&backup.directory).map_err(|_| DiskError::RecoveryRequired)?;
        if backup
            .observe()
            .map_err(|_| DiskError::RecoveryRequired)?
            .as_ref()
            != Some(expected)
        {
            return Err(DiskError::RecoveryRequired);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn rename_exclusive(directory: &Directory, from: &str, to: &str) -> Result<(), DiskError> {
    use std::{ffi::CString, os::fd::AsRawFd};
    let from = CString::new(from).map_err(|_| DiskError::Invalid)?;
    let to = CString::new(to).map_err(|_| DiskError::Invalid)?;
    use std::os::fd::AsFd;
    // SAFETY: both live, NUL-terminated names and the borrowed directory handle
    // outlive the synchronous call. Names are single components produced or
    // validated by DiskTarget; RENAME_EXCL prevents replacing an occupied target.
    let result = unsafe {
        nix::libc::renameatx_np(
            directory.as_fd().as_raw_fd(),
            from.as_ptr(),
            directory.as_fd().as_raw_fd(),
            to.as_ptr(),
            nix::libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        return Ok(());
    }
    match nix::errno::Errno::last() {
        nix::errno::Errno::EEXIST => Err(DiskError::Conflict),
        // Do not fall back to overwrite-capable rename when unsupported.
        nix::errno::Errno::ENOSYS | nix::errno::Errno::ENOTSUP => {
            Err(DiskError::UnsupportedPlatform)
        }
        _ => Err(DiskError::RecoveryRequired),
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn rename_exclusive(directory: &Directory, from: &str, to: &str) -> Result<(), DiskError> {
    nix::fcntl::renameat2(
        directory,
        from,
        directory,
        to,
        nix::fcntl::RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| {
        if error == nix::errno::Errno::EEXIST {
            DiskError::Conflict
        } else {
            DiskError::RecoveryRequired
        }
    })
}

#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
fn rename_exclusive(_: &Directory, _: &str, _: &str) -> Result<(), DiskError> {
    Err(DiskError::UnsupportedPlatform)
}
