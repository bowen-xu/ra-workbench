//! Filesystem and index operations shared by forward and inverse plans.
use super::*;
use crate::agent::workspace::{DiskTarget, ProposalObjects};
use crate::agent::writing_workspace::index::FileIndex;
use crate::ai::structured_resource_validation::Flow;
use research_agent_core::workspace::{FileChange, FileVersion};
use std::path::PathBuf;

pub(super) struct PreparedDiskChange {
    index: FileIndex,
    target: DiskTarget,
    before: Option<FileVersion>,
    after: Option<FileVersion>,
    bytes: Option<Vec<u8>>,
}

pub(super) fn prepare_with_index<P>(
    store: &AppStore,
    tx: &rusqlite::Transaction<'_>,
    manuscript: Uuid,
    change: &FileChange,
    capture_index: impl FnOnce(&str) -> Result<FileIndex, Flow<Error, P>>,
) -> Result<PreparedDiskChange, Flow<Error, P>> {
    let root = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Flow::original(Error::Storage))?;
    let index = capture_index(&change.path)?;
    prepare_in_root(store, tx, manuscript, change, index, root).map_err(Flow::original)
}

fn prepare_in_root(
    store: &AppStore,
    tx: &rusqlite::Transaction<'_>,
    manuscript: Uuid,
    change: &FileChange,
    index: FileIndex,
    root: PathBuf,
) -> Result<PreparedDiskChange, Error> {
    let path = &change.path;
    let before = change.before.clone();
    let after = change.after.clone();
    if let Some(before) = &before {
        index
            .check_registered(tx, manuscript, path, before)
            .map_err(|cause| cause.map(|_| LeaseError::Conflict.into()))?;
    } else {
        index
            .check_unregistered(tx, manuscript, path)
            .map_err(|cause| cause.map(|_| LeaseError::Conflict.into()))?;
        store
            .check_absent_writing_file(&root, manuscript, path)
            .map_err(|_| LeaseError::Conflict)?;
    }
    let source = root.join(manuscript.to_string()).join("source");
    let target = DiskTarget::open(&source, path).map_err(|_| LeaseError::Conflict)?;
    if target.observe().map_err(|_| LeaseError::Conflict)? != before {
        return Err(LeaseError::Conflict.into());
    }
    let objects = ProposalObjects::open(
        &store
            .content_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?,
    )?;
    let bytes = if let Some(version) = &after {
        let object_path: String = tx
            .query_row(
                "select relative_path from content_objects where sha256=?1",
                [&version.sha256],
                |r| r.get(0),
            )
            .map_err(|_| Error::Storage)?;
        Some(objects.read_registered(version, &object_path)?)
    } else {
        None
    };
    Ok(PreparedDiskChange {
        index,
        target,
        before,
        after,
        bytes,
    })
}

impl PreparedDiskChange {
    pub(super) fn index(&self) -> FileIndex {
        self.index.clone()
    }
    pub(super) fn execute(self, operation: Uuid) -> Result<Option<FileVersion>, Error> {
        let Self {
            index: _,
            target,
            before,
            after,
            bytes,
        } = self;
        Ok(match (&before, &after, &bytes) {
            (None, Some(after), Some(bytes)) => Some(
                target
                    .create(operation, bytes, after)
                    .map_err(|_| Error::RecoveryRequired)?,
            ),
            (Some(before), Some(after), Some(bytes)) => Some(
                target
                    .replace(operation, before, bytes, after)
                    .map_err(|_| Error::RecoveryRequired)?,
            ),
            (Some(before), None, None) => {
                target
                    .remove(operation, before)
                    .map_err(|_| Error::RecoveryRequired)?;
                None
            }
            _ => return Err(Error::RecoveryRequired),
        })
    }
}

pub(super) fn update_index_with_source(
    index: &FileIndex,
    tx: &rusqlite::Transaction<'_>,
    manuscript: Uuid,
    path: &str,
    before: Option<&FileVersion>,
    actual: Option<&FileVersion>,
) -> Result<(), Error> {
    if let Some(before) = before {
        index
            .update_registered(tx, manuscript, path, before, actual)
            .map_err(|cause| cause.map(|_| Error::Storage))?;
    } else {
        index
            .register_created(tx, manuscript, path, actual.ok_or(LeaseError::Conflict)?)
            .map_err(|cause| cause.map(|_| Error::Storage))?;
    }
    Ok(())
}
