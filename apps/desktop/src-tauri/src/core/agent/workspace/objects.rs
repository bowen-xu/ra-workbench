//! Immutable object bytes for proposal publication. The caller must hold the
//! AppStore content-write lock through verification and the metadata commit.
//! This layer never publishes database references or deletes existing objects.
use super::{LeaseError, storage};
use crate::agent::instructions::directory::Directory;
use research_agent_core::workspace::{FileVersion, MAX_FILE_BYTES};
use sha2::{Digest, Sha256};
use std::{io::Read, os::unix::fs::MetadataExt, path::Path};

pub struct ProposalObjects {
    root: Directory,
}
impl ProposalObjects {
    /// Full verified bytes for native application, including binary files. Caller
    /// holds the content read lock; the review/UI size limit does not apply.
    pub fn read_registered(
        &self,
        version: &FileVersion,
        relative: &str,
    ) -> Result<Vec<u8>, LeaseError> {
        self.verify_registered(version, relative)?;
        let parts: Vec<_> = relative.split('/').collect();
        let first = self.root.child(Path::new(parts[0]))?;
        let second = first.child(Path::new(parts[1]))?;
        if first.identity()?.0 != self.root.identity()?.0
            || second.identity()?.0 != self.root.identity()?.0
        {
            return Err(LeaseError::Invalid);
        }
        let file = second.config_file(parts[2])?.ok_or(LeaseError::Conflict)?;
        if file.metadata()?.nlink() != 1 {
            return Err(LeaseError::Conflict);
        }
        let mut bytes = Vec::new();
        file.take(version.byte_size + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != version.byte_size
            || format!("{:x}", Sha256::digest(&bytes)) != version.sha256
        {
            return Err(LeaseError::Conflict);
        }
        Ok(bytes)
    }
    /// Bounded UI content from an immutable registered object. Never read the
    /// execution copy, and never replace failed verification with newer bytes.
    pub fn review_registered(
        &self,
        version: &FileVersion,
        relative: &str,
    ) -> Result<super::ReviewContent, LeaseError> {
        const MAX_TEXT_BYTES: u64 = 1024 * 1024;
        self.verify_registered(version, relative)?;
        if version.byte_size > MAX_TEXT_BYTES {
            return Ok(super::ReviewContent::TooLarge);
        }
        // Re-open using the same validated handle-relative grammar, then check
        // the returned bytes too: verification is not a promise about a later read.
        let parts: Vec<_> = relative.split('/').collect();
        let first = self.root.child(Path::new(parts[0]))?;
        let second = first.child(Path::new(parts[1]))?;
        if first.identity()?.0 != self.root.identity()?.0
            || second.identity()?.0 != self.root.identity()?.0
        {
            return Err(LeaseError::Invalid);
        }
        let file = second.config_file(parts[2])?.ok_or(LeaseError::Conflict)?;
        if file.metadata()?.nlink() != 1 {
            return Err(LeaseError::Conflict);
        }
        let mut bytes = Vec::new();
        file.take(MAX_TEXT_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != version.byte_size
            || format!("{:x}", Sha256::digest(&bytes)) != version.sha256
        {
            return Err(LeaseError::Conflict);
        }
        if bytes.contains(&0) {
            return Ok(super::ReviewContent::Binary);
        }
        Ok(match String::from_utf8(bytes) {
            Ok(text) => super::ReviewContent::Text(text),
            Err(_) => super::ReviewContent::Binary,
        })
    }

    /// Root is the application's canonical content directory, never model input.
    pub fn open(root: &Path) -> Result<Self, LeaseError> {
        Ok(Self {
            root: Directory::root(root)?,
        })
    }

    /// Stage verified bytes at the content-addressed path. Existing bytes are
    /// verified, never overwritten. A failed/crashed new write can leave an
    /// unreferenced file; the publication/recovery host owns its reconciliation.
    /// In particular, callers must not infer success from path existence.
    pub fn stage(&self, version: &FileVersion, bytes: &[u8]) -> Result<String, LeaseError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(LeaseError::Budget);
        }
        if version.byte_size != bytes.len() as u64
            || version.sha256 != format!("{:x}", Sha256::digest(bytes))
        {
            return Err(LeaseError::Conflict);
        }
        let sha = &version.sha256;
        let first = self.directory(&self.root, &sha[..2])?;
        let second = self.directory(&first, &sha[2..4])?;
        match second.config_file(sha)? {
            Some(file) => verify(file, version)?,
            None => {
                // O_EXCL and O_NOFOLLOW prohibit replacing a competing object.
                // sync_all(file) and fsync(parent) precede metadata publication.
                storage::create_file(&second, sha, bytes, false)?;
                verify(
                    second.config_file(sha)?.ok_or(LeaseError::Conflict)?,
                    version,
                )?;
            }
        }
        storage::sync(&second)?;
        storage::sync(&first)?;
        storage::sync(&self.root)?;
        Ok(format!("{}/{}/{}", &sha[..2], &sha[2..4], sha))
    }

    /// Validate a previously registered path, including legacy extension-bearing
    /// objects. It must remain inside its hash directories and name the hash.
    /// Existing MIME/path metadata belongs to its original importer.
    pub fn verify_registered(
        &self,
        version: &FileVersion,
        relative: &str,
    ) -> Result<(), LeaseError> {
        let sha = &version.sha256;
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err(LeaseError::Invalid);
        }
        let parts: Vec<_> = relative.split('/').collect();
        if parts.len() != 3
            || parts[0] != &sha[..2]
            || parts[1] != &sha[2..4]
            || !(parts[2] == sha
                || parts[2].strip_prefix(sha).is_some_and(|ext| {
                    ext.starts_with('.')
                        && ext.len() > 1
                        && ext[1..].bytes().all(|v| v.is_ascii_alphanumeric())
                }))
        {
            return Err(LeaseError::Invalid);
        }
        let first = self.root.child(Path::new(parts[0]))?;
        let second = first.child(Path::new(parts[1]))?;
        if first.identity()?.0 != self.root.identity()?.0
            || second.identity()?.0 != self.root.identity()?.0
        {
            return Err(LeaseError::Invalid);
        }
        verify(
            second.config_file(parts[2])?.ok_or(LeaseError::Conflict)?,
            version,
        )?;
        storage::sync(&second)?;
        storage::sync(&first)?;
        storage::sync(&self.root)
    }

    fn directory(&self, parent: &Directory, name: &str) -> Result<Directory, LeaseError> {
        let child = match parent.optional_child(Path::new(name))? {
            Some(child) => child,
            None => storage::create_directory(parent, name)?,
        };
        if child.identity()?.0 != self.root.identity()?.0 {
            return Err(LeaseError::Invalid);
        }
        Ok(child)
    }
}
fn verify(file: std::fs::File, version: &FileVersion) -> Result<(), LeaseError> {
    let metadata = file.metadata()?;
    if metadata.len() != version.byte_size
        || metadata.nlink() != 1
        || version.byte_size > MAX_FILE_BYTES
    {
        return Err(LeaseError::Conflict);
    }
    let mut reader = file.take(version.byte_size + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut size = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hash.update(&buffer[..read]);
    }
    if size != version.byte_size || format!("{:x}", hash.finalize()) != version.sha256 {
        return Err(LeaseError::Conflict);
    }
    reader.get_ref().sync_all()?;
    Ok(())
}

#[cfg(test)]
#[path = "objects/tests.rs"]
mod tests;
