//! Shared physical compensation; callers own committed intent and journal ACK.
use super::*;
use crate::agent::workspace::DiskTarget;
use crate::agent::writing_workspace::index::{FileIndex, IndexFailure};
use research_agent_core::workspace::{FileChange, FileVersion};

pub(super) struct DiskRestoration {
    index: FileIndex,
    indexed: Option<FileVersion>,
    check_only: bool,
    pub actual: Option<FileVersion>,
}
impl DiskRestoration {
    /// Commit the index and caller's restoration receipt in the same transaction.
    pub fn update_index(
        &self,
        tx: &rusqlite::Transaction<'_>,
        manuscript: Uuid,
        path: &str,
    ) -> Result<(), Error> {
        if self.check_only {
            self.index
                .check_registered(
                    tx,
                    manuscript,
                    path,
                    self.actual.as_ref().ok_or(LeaseError::Conflict)?,
                )
                .map_err(|cause| cause.map(|_| LeaseError::Conflict.into()))?;
            return Ok(());
        }
        match (&self.indexed, &self.actual) {
            (Some(before), actual) => self
                .index
                .update_registered(tx, manuscript, path, before, actual.as_ref())
                .map_err(|cause| cause.map(|_| Error::Storage))?,
            (None, Some(actual)) => self
                .index
                .register_created(tx, manuscript, path, actual)
                .map_err(|cause| cause.map(|_| Error::Storage))?,
            (None, None) => self
                .index
                .check_unregistered(tx, manuscript, path)
                .map_err(|cause| cause.map(|_| Error::Storage))?,
        }
        Ok(())
    }
}

/// The connection mutex must remain held across physical work and the ACK tx.
/// Uses this operation's preserved inode/creation proof, never a blind overwrite.
pub(super) fn restore(
    store: &AppStore,
    db: &rusqlite::Connection,
    manuscript: Uuid,
    operation: Uuid,
    change: &FileChange,
    index: Result<FileIndex, Error>,
) -> Result<DiskRestoration, Error> {
    let added = change.before.is_none();
    let replaced = change.before.is_some() && change.after.is_some();
    let version = if added {
        change.after.as_ref()
    } else {
        change.before.as_ref()
    }
    .ok_or(LeaseError::Conflict)?;
    let source = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Error::Storage)?
        .join(manuscript.to_string())
        .join("source");
    let index = index?;
    let absent = index_matches(index.check_unregistered(db, manuscript, &change.path))?;
    if replaced && absent {
        return Err(LeaseError::Conflict.into());
    }
    let indexed = if absent {
        None
    } else if index_matches(index.check_registered(db, manuscript, &change.path, version))? {
        Some(version.clone())
    } else {
        if !replaced {
            return Err(LeaseError::Conflict.into());
        }
        let after = change.after.as_ref().ok_or(LeaseError::Conflict)?;
        index
            .check_registered(db, manuscript, &change.path, after)
            .map_err(|cause| cause.map(|_| LeaseError::Conflict.into()))?;
        Some(after.clone())
    };
    let target = DiskTarget::open(&source, &change.path).map_err(|_| Error::RecoveryRequired)?;
    let actual = if added {
        target
            .rollback_added(operation, version)
            .map_err(|_| Error::RecoveryRequired)?;
        None
    } else if replaced {
        Some(
            target
                .rollback_replaced(
                    operation,
                    version,
                    change.after.as_ref().ok_or(LeaseError::Conflict)?,
                )
                .map_err(|_| Error::RecoveryRequired)?,
        )
    } else {
        Some(
            target
                .restore_removed(operation, version)
                .map_err(|_| Error::RecoveryRequired)?,
        )
    };
    Ok(DiskRestoration {
        index,
        indexed,
        actual,
        check_only: !added && !replaced && !absent,
    })
}

/// Validate every frozen disk target before the first compensation mutates any file.
// Keep the existing physical/receipt proof arguments explicit while borrowing
// the application-owned domain adapter for the same database snapshot.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_plan(
    index: &FileIndex,
    tx: &rusqlite::Connection,
    manuscript: Uuid,
    operation: Uuid,
    target: &DiskTarget,
    actual: &Option<FileVersion>,
    change: &FileChange,
    phase: crate::agent::repository::proposal_applications::ApplicationPhase,
) -> Result<(), Error> {
    use crate::agent::repository::proposal_applications::ApplicationPhase;
    let index_matches =
        |version: &Option<research_agent_core::workspace::FileVersion>| match version {
            Some(v) => index_matches(index.check_registered(tx, manuscript, &change.path, v)),
            None => index_matches(index.check_unregistered(tx, manuscript, &change.path)),
        };
    if matches!(
        phase,
        ApplicationPhase::Pending | ApplicationPhase::Restored
    ) {
        if *actual != change.before || !index_matches(&change.before)? {
            return Err(LeaseError::Conflict.into());
        }
    } else {
        if !index_matches(&change.before)? && !index_matches(&change.after)? {
            return Err(LeaseError::Conflict.into());
        }
        // A replacement can stop between displacing the result and
        // restoring its backup. Absence is allowed only with proof.
        let preserved = target
            .observe_preserved(operation)
            .map_err(|_| Error::RecoveryRequired)?;
        if preserved.as_ref().is_some_and(|version| {
            Some(version) != change.before.as_ref().or(change.after.as_ref())
        }) {
            return Err(LeaseError::Conflict.into());
        }
        if *actual != change.before
            && *actual != change.after
            && !(actual.is_none() && change.before.is_some() && preserved == change.before)
        {
            return Err(LeaseError::Conflict.into());
        }
        if let Some(before) = &change.before {
            if actual.as_ref() != Some(before) && preserved.as_ref() != Some(before) {
                return Err(LeaseError::Conflict.into());
            }
        } else if actual.is_none() && preserved != change.after {
            return Err(LeaseError::Conflict.into());
        }
    }
    Ok(())
}

fn index_matches(result: Result<(), IndexFailure>) -> Result<bool, Error> {
    match result {
        Ok(()) => Ok(true),
        Err(IndexFailure::Original(_)) => Ok(false),
        Err(cause) => Err(cause.map(|_| LeaseError::Conflict.into())),
    }
}
