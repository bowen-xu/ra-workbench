//! Private temporary resources are leased to one Core catalog. Reopening a
//! catalog collects only abandoned sessions in this application's namespace.
use super::super::RuntimePluginCatalog;
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};

const NAMESPACE: &str = "research-assistant-native-v1";
const OWNER: &str = "owner.lock";
const REGISTRY: &str = "registry.lock";
const ENTRY_LIMIT: usize = 4096;

pub(crate) struct StagingSession {
    path: PathBuf,
    root: PathBuf,
    lock: File,
}

pub(super) fn for_catalog(catalog: &RuntimePluginCatalog) -> io::Result<Arc<StagingSession>> {
    let mut session = catalog
        .native_staging
        .lock()
        .map_err(|_| io::Error::other("native staging owner unavailable"))?;
    if let Some(session) = session.as_ref() {
        session.validate()?;
        return Ok(Arc::clone(session));
    }
    let created = Arc::new(StagingSession::open_in(
        &std::env::temp_dir().canonicalize()?,
    )?);
    *session = Some(Arc::clone(&created));
    Ok(created)
}

impl StagingSession {
    fn open_in(parent: &Path) -> io::Result<Self> {
        let root = parent.join(NAMESPACE);
        match fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        private_directory(&root)?;
        let registry = private_lock(&root.join(REGISTRY))?;
        registry.lock_exclusive()?;
        collect_abandoned(&root)?;
        let path = root.join(format!("session-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        let lock = private_lock(&path.join(OWNER))?;
        lock.try_lock_exclusive()?;
        // The registry serializes publication of the directory and owner lock.
        // A crash before the lock exists leaves only an empty directory.
        Ok(Self { path, root, lock })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    fn validate(&self) -> io::Result<()> {
        private_directory(&self.root)?;
        private_directory(&self.path)?;
        let actual = fs::symlink_metadata(self.path.join(OWNER))?;
        let expected = self.lock.metadata()?;
        if (actual.dev(), actual.ino()) != (expected.dev(), expected.ino()) {
            return Err(invalid());
        }
        Ok(())
    }
}

impl Drop for StagingSession {
    fn drop(&mut self) {
        // TempDir owners normally removed their children first. Retained inputs
        // or executable images must survive uncertain process settlement until
        // the entire Core owner exits, even if another profile stages a host.
        let cleanup = || -> io::Result<()> {
            self.validate()?;
            let registry = private_lock(&self.root.join(REGISTRY))?;
            registry.try_lock_exclusive()?;
            let entries = entries(&self.path)?;
            if entries.len() == 1 && entries[0].file_name() == OWNER {
                fs::remove_file(self.path.join(OWNER))?;
                fs::remove_dir(&self.path)?;
            }
            Ok(())
        };
        if let Err(error) = cleanup() {
            eprintln!("Native staging owner cleanup deferred: {:?}", error.kind());
        }
        // Match the existing workspace owner: a concurrently forked child may
        // briefly inherit the file description before its CLOEXEC takes effect.
        if let Err(error) = FileExt::unlock(&self.lock) {
            eprintln!("Native staging ownership unlock failed: {:?}", error.kind());
        }
    }
}

fn collect_abandoned(root: &Path) -> io::Result<()> {
    for entry in entries(root)? {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_prefix("session-")) else {
            continue;
        };
        if !uuid::Uuid::parse_str(id).is_ok_and(|id_value| id_value.to_string() == id)
            || !entry.file_type()?.is_dir()
        {
            continue;
        }
        let path = entry.path();
        private_directory(&path)?;
        let owner_path = path.join(OWNER);
        let lock = match existing_lock(&owner_path) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Only a partially published, empty session lacks an owner.
                fs::remove_dir(&path)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        match lock.try_lock_exclusive() {
            Ok(()) => {
                // std's Unix removal does not follow symlinks, including links
                // inside abandoned input directories. Never scan OS temp peers.
                validate_contents(&path)?;
                remove_abandoned(&path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn remove_abandoned(path: &Path) -> io::Result<()> {
    // Retain the ownership marker until every child is gone. If removal fails
    // partway, the next recovery can still qualify and retry this same session.
    for entry in entries(path)? {
        if entry.file_name() == OWNER {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    fs::remove_file(path.join(OWNER))?;
    fs::remove_dir(path)
}

fn validate_contents(path: &Path) -> io::Result<()> {
    let device = fs::symlink_metadata(path)?.dev();
    let mut count = 0;
    for entry in entries(path)? {
        if entry.file_name() == OWNER {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(invalid)?;
        if !name.starts_with("research-native-host-") && !name.starts_with("research-native-input-")
        {
            return Err(invalid());
        }
        // A link itself can be unlinked safely, but never traverse it.
        if !entry.file_type()?.is_dir() {
            continue;
        }
        private_directory(&entry.path())?;
        if entry.metadata()?.dev() != device {
            return Err(invalid());
        }
        for file in entries(&entry.path())? {
            count += 1;
            if count > ENTRY_LIMIT || file.file_type()?.is_dir() {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn private_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
        || path.canonicalize()? != path
    {
        return Err(invalid());
    }
    Ok(())
}

fn private_lock(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK)
        .open(path)?;
    validate_lock(file)
}

fn existing_lock(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK)
        .open(path)?;
    validate_lock(file)
}

fn validate_lock(file: File) -> io::Result<File> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(invalid());
    }
    Ok(file)
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no pointers and has no caller preconditions.
    unsafe { nix::libc::geteuid() }
}

fn entries(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        if entries.len() == ENTRY_LIMIT {
            return Err(io::Error::other("native staging entry limit"));
        }
        entries.push(entry?);
    }
    Ok(entries)
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "native staging ownership changed",
    )
}

#[cfg(test)]
mod tests;
