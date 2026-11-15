use super::{LeaseError, storage, tree};
use crate::agent::instructions::directory::Directory;
use research_agent_core::workspace::{FileChange, WorkspaceIdentity, WorkspaceManifest, compare};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct WorkspaceInput {
    pub path: String,
    pub bytes: Vec<u8>,
}
/// Independent of instruction trust: the host must authorize app-owned storage.
pub struct StorageIdentity<'a> {
    pub path: &'a Path,
    pub device: u64,
    pub inode: u64,
}
pub struct WorkspaceStore {
    pub(super) root: Arc<Directory>,
    path: PathBuf,
}
pub struct WorkspaceLease {
    root: Arc<Directory>,
    directory: Directory,
    base: Directory,
    work: Directory,
    path: PathBuf,
    manifest: WorkspaceManifest,
    _lock: Arc<OwnershipLock>,
}

/// A bounded text edit requested by the native patch tool. `before` is an
/// optimistic concurrency check: an edit is rejected when the copy no longer
/// contains the bytes the model saw.
pub struct TextPatchOperation {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}
/// Release the flock explicitly when the last application owner finishes. A
/// concurrently spawned child may briefly retain a duplicated descriptor until
/// exec; closing only the parent descriptor would incorrectly retain its lock.
pub(super) struct OwnershipLock(pub(super) File, pub(super) std::sync::atomic::AtomicBool);
impl Drop for OwnershipLock {
    fn drop(&mut self) {
        if !self.1.load(std::sync::atomic::Ordering::Acquire)
            && fs2::FileExt::unlock(&self.0).is_err()
        {
            // Drop still closes our descriptor. An inherited descriptor may keep
            // the lock until exit; subsequent recovery must continue to fail busy.
            eprintln!("Agent workspace ownership unlock failed; recovery may remain busy");
        }
    }
}
impl WorkspaceStore {
    /// Open existing app-owned storage. No child is inspected before authorization.
    pub fn open(
        path: &Path,
        authorize: impl FnOnce(&StorageIdentity<'_>) -> Result<(), LeaseError>,
    ) -> Result<Self, LeaseError> {
        if !path.is_absolute() || path.canonicalize()? != path {
            return Err(LeaseError::Invalid);
        }
        let root = Directory::root(path)?;
        let (device, inode) = root.identity()?;
        authorize(&StorageIdentity {
            path,
            device,
            inode,
        })?;
        Ok(Self {
            root: Arc::new(root),
            path: path.into(),
        })
    }
    /// Populate only newly created private directories. Inputs must already be
    /// frozen and authorized by the host; no original manuscript paths are read.
    pub fn create(
        &self,
        manifest: WorkspaceManifest,
        inputs: impl IntoIterator<Item = WorkspaceInput>,
    ) -> Result<WorkspaceLease, LeaseError> {
        manifest.validate()?;
        let encoded = serde_json::to_vec(&manifest).map_err(|_| LeaseError::Invalid)?;
        if encoded.len() > 32 * 1024 * 1024 {
            return Err(LeaseError::Budget);
        }
        let name = manifest.identity.lease_id.to_string();
        if self
            .root
            .config_file(&super::discard::journal_name(&manifest.identity))?
            .is_some()
            || self
                .root
                .config_file(&super::discard::receipt_name(&manifest.identity))?
                .is_some()
        {
            return Err(LeaseError::Conflict);
        }
        let directory = storage::create_directory(&self.root, &name)?;
        let result = (|| {
            let lock = storage::create_file(&directory, "owner.lock", b"", false)?;
            fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| LeaseError::Busy)?;
            let lock = OwnershipLock(lock, Default::default());
            storage::create_file(&directory, "manifest.json", &encoded, false)?;
            let base = storage::create_directory(&directory, "base")?;
            let work = storage::create_directory(&directory, "work")?;
            let wanted: BTreeMap<_, _> = manifest
                .files
                .iter()
                .map(|entry| (entry.file.path.as_str(), &entry.file.version))
                .collect();
            let mut written = BTreeSet::new();
            for input in inputs {
                let expected = wanted.get(input.path.as_str()).ok_or(LeaseError::Invalid)?;
                if !written.insert(input.path.clone()) {
                    return Err(LeaseError::Invalid);
                }
                if input.bytes.len() as u64 != expected.byte_size
                    || tree::version(&input.bytes, expected.executable) != **expected
                {
                    return Err(LeaseError::Conflict);
                }
                storage::write_path(&base, &input.path, &input.bytes, expected.executable)?;
                storage::write_path(&work, &input.path, &input.bytes, expected.executable)?;
            }
            if written.len() != wanted.len() {
                return Err(LeaseError::Incomplete);
            }
            // Creation may normalize names on the target filesystem. Never
            // publish a baseline whose actual path inventory differs from it.
            if !compare(&manifest, &tree::scan(&base)?)?.is_empty()
                || !compare(&manifest, &tree::scan(&work)?)?.is_empty()
            {
                return Err(LeaseError::Conflict);
            }
            storage::create_file(&directory, "ready", b"1", false)?;
            Ok((lock, base, work))
        })();
        match result {
            Ok((lock, base, work)) => Ok(WorkspaceLease {
                root: self.root.clone(),
                directory,
                base,
                work,
                path: self.path.join(name),
                manifest,
                _lock: Arc::new(lock),
            }),
            Err(error) => {
                if storage::remove_tree(&self.root, OsStr::new(&name), &mut 100_000, 0).is_err() {
                    return Err(LeaseError::CleanupRequired);
                }
                Err(error)
            }
        }
    }
    pub fn restore(&self, expected: &WorkspaceIdentity) -> Result<WorkspaceLease, LeaseError> {
        if self
            .root
            .config_file(&super::discard::journal_name(expected))?
            .is_some()
            || self
                .root
                .config_file(&super::discard::receipt_name(expected))?
                .is_some()
        {
            return Err(LeaseError::Incomplete);
        }
        let (directory, lock, manifest) = self.open_owned(expected)?;
        if directory.config_file("deleting")?.is_some()
            || storage::read_file(&directory, "ready", 1)? != b"1"
        {
            return Err(LeaseError::Incomplete);
        }
        let base = directory.child(Path::new("base"))?;
        let work = directory.child(Path::new("work"))?;
        if !compare(&manifest, &tree::scan(&base)?)?.is_empty() {
            return Err(LeaseError::Conflict);
        }
        Ok(WorkspaceLease {
            root: self.root.clone(),
            directory,
            base,
            work,
            path: self.path.join(expected.lease_id.to_string()),
            manifest,
            _lock: Arc::new(lock),
        })
    }
    pub(super) fn open_owned(
        &self,
        expected: &WorkspaceIdentity,
    ) -> Result<(Directory, OwnershipLock, WorkspaceManifest), LeaseError> {
        if expected.lease_id.is_nil() {
            return Err(LeaseError::Invalid);
        }
        let directory = self
            .root
            .optional_child(Path::new(&expected.lease_id.to_string()))?
            .ok_or(LeaseError::Incomplete)?;
        let lock = directory
            .config_file("owner.lock")?
            .ok_or(LeaseError::Incomplete)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                LeaseError::Busy
            } else {
                LeaseError::Unavailable
            }
        })?;
        let lock = OwnershipLock(lock, Default::default());
        if directory.config_file(super::execution::MARKER)?.is_some() {
            return Err(LeaseError::Incomplete);
        }
        let bytes = storage::read_file(&directory, "manifest.json", 32 * 1024 * 1024)?;
        let manifest: WorkspaceManifest =
            serde_json::from_slice(&bytes).map_err(|_| LeaseError::Invalid)?;
        manifest.validate()?;
        if &manifest.identity != expected {
            return Err(LeaseError::Conflict);
        }
        Ok((directory, lock, manifest))
    }
    /// Caller must stop processes first. No automatic deletion on Drop: an
    /// interrupted app must retain the working copy for recovery or review.
    /// Can retry after interruption or lost completion; permanent parent receipts
    /// prevent recreation/restore of a successfully discarded lease identity.
    pub fn discard(&self, expected: &WorkspaceIdentity) -> Result<(), LeaseError> {
        super::discard::discard(self, expected)
    }
    /// Identity-validated journal state; does not authorize or perform deletion.
    pub fn discard_state(
        &self,
        expected: &WorkspaceIdentity,
    ) -> Result<Option<super::DiscardState>, LeaseError> {
        super::discard::state(self, expected)
    }
}
impl WorkspaceLease {
    /// The native caller must first revoke the durable execution grant. Keep
    /// read handles alive but allow the frozen delivery reader to acquire its
    /// own lock. Active/uncertain processes still prevent this handoff.
    pub(crate) fn release_for_delivery(&self) -> Result<(), LeaseError> {
        if self._lock.1.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        self.changes()?;
        fs2::FileExt::unlock(&self._lock.0)?;
        self._lock
            .1
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    /// After database intent commit, freeze the exact reviewed inventory for
    /// deletion before releasing ownership. Never removes files itself.
    pub fn prepare_discard(self, expected: &[FileChange]) -> Result<(), LeaseError> {
        if self._lock.1.load(std::sync::atomic::Ordering::Acquire) {
            return Err(LeaseError::Unauthorized);
        }
        if self.changes()? != expected {
            return Err(LeaseError::Conflict);
        }
        super::discard::mark(&self.root, &self.directory, &self.manifest.identity)
    }
    /// Persist before launching a writer. Blocking-worker only. The returned
    /// guard retains exclusive ownership even if the caller drops this lease.
    pub fn begin_execution(&mut self) -> Result<super::WorkspaceExecution, LeaseError> {
        let work = self.working_directory()?;
        let directory = Directory::root(&self.path)?;
        if directory.identity()? != self.directory.identity()? {
            return Err(LeaseError::Conflict);
        }
        super::WorkspaceExecution::begin(directory, self._lock.clone(), work)
    }
    pub fn manifest(&self) -> &WorkspaceManifest {
        &self.manifest
    }
    /// For a quiescent worktree only. Process supervision must stop writers first.
    pub fn changes(&self) -> Result<Vec<FileChange>, LeaseError> {
        if self
            .directory
            .config_file(super::execution::MARKER)?
            .is_some()
        {
            return Err(LeaseError::Busy);
        }
        storage::same_child(
            &self.root,
            OsStr::new(&self.manifest.identity.lease_id.to_string()),
            &self.directory,
        )?;
        storage::same_child(&self.directory, OsStr::new("base"), &self.base)?;
        storage::same_child(&self.directory, OsStr::new("work"), &self.work)?;
        if !compare(&self.manifest, &tree::scan(&self.base)?)?.is_empty() {
            return Err(LeaseError::Conflict);
        }
        Ok(compare(&self.manifest, &tree::scan(&self.work)?)?)
    }
    /// Revalidate before returning a display/spawn path. A sandbox/launcher must
    /// still bind the actual execution cwd to this lease; this is not a capability.
    pub fn working_directory(&self) -> Result<PathBuf, LeaseError> {
        if self._lock.1.load(std::sync::atomic::Ordering::Acquire) {
            return Err(LeaseError::Unauthorized);
        }
        let name = self.manifest.identity.lease_id.to_string();
        storage::same_child(&self.root, OsStr::new(&name), &self.directory)?;
        let path = self.path.join("work");
        let current = Directory::root(&path)?;
        if current.identity()? != self.work.identity()? {
            return Err(LeaseError::Conflict);
        }
        Ok(path)
    }

    /// Apply validated text operations to the approved copy. This is the native
    pub(crate) fn read_text(&self, path: &str) -> Result<Option<String>, LeaseError> {
        research_agent_core::workspace::validate_path(path)?;
        storage::read_path(&self.work, path, 16 * 1024 * 1024)?
            .map(|bytes| {
                if bytes.contains(&0) {
                    return Err(LeaseError::Invalid);
                }
                String::from_utf8(bytes).map_err(|_| LeaseError::Invalid)
            })
            .transpose()
    }

    /// Apply validated text operations to the approved copy. This is the native
    /// equivalent of Codex's apply_patch: it never resolves a model path against
    /// the host filesystem and it leaves the live manuscript untouched.
    pub fn apply_text_operations(
        &mut self,
        operations: &[TextPatchOperation],
    ) -> Result<Vec<FileChange>, LeaseError> {
        if self._lock.1.load(std::sync::atomic::Ordering::Acquire) {
            return Err(LeaseError::Unauthorized);
        }
        if operations.is_empty() || operations.len() > 128 {
            return Err(LeaseError::Invalid);
        }
        let mut paths = BTreeSet::new();
        let mut writes = Vec::with_capacity(operations.len());
        for operation in operations {
            research_agent_core::workspace::validate_path(&operation.path)?;
            if !paths.insert(operation.path.clone()) {
                return Err(LeaseError::Invalid);
            }
            let current = storage::read_path(&self.work, &operation.path, 16 * 1024 * 1024)?;
            let current_text = current
                .as_deref()
                .map(str::from_utf8)
                .transpose()
                .map_err(|_| LeaseError::Invalid)?;
            match (&current, &operation.after) {
                (None, None) => return Err(LeaseError::Invalid),
                (None, Some(after)) => writes.push((
                    operation.path.clone(),
                    after.as_bytes().to_vec(),
                    false,
                    false,
                )),
                (Some(_), None) => {
                    if current_text != operation.before.as_deref() {
                        return Err(LeaseError::ContentMismatch);
                    }
                    writes.push((operation.path.clone(), Vec::new(), true, true));
                }
                (Some(_), Some(after)) => {
                    let before = operation
                        .before
                        .as_deref()
                        .ok_or(LeaseError::ContentMismatch)?;
                    let current = current_text.ok_or(LeaseError::Invalid)?;
                    let next = replace_patch_context(current, before, after)?;
                    writes.push((operation.path.clone(), next.into_bytes(), false, true));
                }
            }
        }
        for (path, bytes, remove, existed) in writes {
            if remove {
                storage::remove_file(&self.work, &path)?;
            } else if existed {
                storage::replace_file(&self.work, &path, &bytes, false)?;
            } else {
                storage::write_path(&self.work, &path, &bytes, false)?;
            }
        }
        self.changes()
    }
}

fn replace_patch_context(current: &str, before: &str, after: &str) -> Result<String, LeaseError> {
    if current == before {
        return Ok(after.to_owned());
    }
    if before.is_empty() {
        return Err(LeaseError::ContentMismatch);
    }
    let mut matches = current.match_indices(before);
    let Some((start, _)) = matches.next() else {
        return Err(LeaseError::ContentMismatch);
    };
    if matches.next().is_some() {
        return Err(LeaseError::AmbiguousPatch);
    }
    let end = start + before.len();
    let mut result = String::with_capacity(current.len() - before.len() + after.len());
    result.push_str(&current[..start]);
    result.push_str(after);
    result.push_str(&current[end..]);
    Ok(result)
}

#[path = "lease/change_bytes.rs"]
mod change_bytes;
#[path = "lease/review.rs"]
mod review;
pub use change_bytes::ChangeSide;
#[cfg(test)]
#[path = "lease/tests.rs"]
mod tests;
pub use review::{ChangeContent, ReviewContent};
