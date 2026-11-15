//! Version-checked byte extraction for a host-owned immutable proposal store.
use super::*;
use research_agent_core::workspace::{FileVersion, MAX_FILE_BYTES};
use std::{io::Read, os::unix::fs::MetadataExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeSide {
    Before,
    After,
}
impl WorkspaceLease {
    /// Read the frozen baseline sides of an externally supplied inventory.
    pub fn visit_baseline_change_bytes(
        &self,
        changes: &[FileChange],
        mut sink: impl FnMut(&FileChange, &[u8]) -> Result<(), LeaseError>,
    ) -> Result<(), LeaseError> {
        let baseline: std::collections::BTreeMap<_, _> = self
            .manifest
            .files
            .iter()
            .map(|entry| (entry.file.path.as_str(), &entry.file.version))
            .collect();
        for change in changes {
            if let Some(version) = &change.before {
                if baseline.get(change.path.as_str()).copied() != Some(version) {
                    return Err(LeaseError::Conflict);
                }
                let bytes = read_version(&self.base, &change.path, version, MAX_FILE_BYTES)?;
                sink(change, &bytes)?;
            }
        }
        Ok(())
    }

    /// Visit the exact current working-copy bytes after the execution marker
    /// has been cleared. This is used when a result must retain the frozen
    /// source that produced an artifact; it never reads the live manuscript.
    pub fn visit_work_bytes(
        &self,
        mut sink: impl FnMut(
            &research_agent_core::workspace::WorkspaceFile,
            &[u8],
        ) -> Result<(), LeaseError>,
    ) -> Result<(), LeaseError> {
        let files = tree::scan(&self.work)?;
        for file in &files {
            let bytes = read_version(&self.work, &file.path, &file.version, MAX_FILE_BYTES)?;
            sink(file, &bytes)?;
        }
        if tree::scan(&self.work)? != files {
            return Err(LeaseError::Conflict);
        }
        Ok(())
    }

    /// Visit present sides of every change in the exact reviewed inventory.
    /// Empty files produce an empty slice; absent sides produce no callback.
    /// Only one file's bytes are resident at a time, including binary files.
    ///
    /// The sink may stage immutable objects, but MUST NOT publish a proposal or
    /// write original sources until this returns Ok. Later failures may follow
    /// successful callbacks. The caller owns orphan reconciliation/rollback.
    /// No execution grant is changed. Call on a blocking worker after writers stop.
    pub fn visit_change_bytes(
        &self,
        expected: &[FileChange],
        mut sink: impl FnMut(&FileChange, ChangeSide, &[u8]) -> Result<(), LeaseError>,
    ) -> Result<(), LeaseError> {
        if self.changes()? != expected {
            return Err(LeaseError::Conflict);
        }
        for change in expected {
            for (side, root, version) in [
                (ChangeSide::Before, &self.base, &change.before),
                (ChangeSide::After, &self.work, &change.after),
            ] {
                if let Some(version) = version {
                    let bytes = read_version(root, &change.path, version, MAX_FILE_BYTES)?;
                    sink(change, side, &bytes)?;
                }
            }
        }
        // Verify the entire inventory again, including absent/new paths and the
        // immutable baseline. A success binds staged bytes to the review, not to
        // whatever the working directory contains at a later acceptance time.
        if self.changes()? != expected {
            return Err(LeaseError::Conflict);
        }
        Ok(())
    }
}

pub(super) fn read_version(
    root: &Directory,
    path: &str,
    expected: &FileVersion,
    limit: u64,
) -> Result<Vec<u8>, LeaseError> {
    research_agent_core::workspace::validate_path(path)?;
    if expected.byte_size > limit {
        return Err(LeaseError::Budget);
    }
    let mut segments: Vec<_> = path.split('/').collect();
    let name = segments.pop().ok_or(LeaseError::Invalid)?;
    let mut parent = root.child(Path::new("."))?;
    for segment in segments {
        let child = parent.child(Path::new(segment))?;
        if child.identity()?.0 != root.identity()?.0 {
            return Err(LeaseError::Invalid);
        }
        parent = child;
    }
    let file = parent.config_file(name)?.ok_or(LeaseError::Conflict)?;
    let metadata = file.metadata()?;
    if metadata.nlink() != 1 || metadata.len() != expected.byte_size {
        return Err(LeaseError::Conflict);
    }
    let mut bytes = vec![];
    file.take(expected.byte_size + 1).read_to_end(&mut bytes)?;
    if tree::version(&bytes, metadata.mode() & 0o111 != 0) != *expected {
        return Err(LeaseError::Conflict);
    }
    Ok(bytes)
}
