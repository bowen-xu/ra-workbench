//! A single inverse disk effect compensated under its existing undo identity.
use super::*;
use crate::agent::repository::{
    proposal_applications::{
        ApplicationPhase, ApplicationScope, ApplicationStatus, ApplicationTarget,
    },
    proposal_undos::{self as undos, StoredUndo},
};
use crate::{agent::writing_workspace::index::FileIndex, ai::structured_resource_validation::Flow};

impl WritingSubmissionService {
    /// Internal coordinator owns complete buffer coverage and manuscript lease.
    /// Completed undo is history, not permission to redo the original proposal.
    pub async fn rollback_disk_undo_file(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        path: String,
    ) -> Result<StoredUndo, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .rollback_undo_file(
                    self.store.clone(),
                    original,
                    operation,
                    path,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            rollback_with_index(&store, original, operation, &path, |_| {
                Ok::<_, Flow<Error, std::convert::Infallible>>(FileIndex::Static(
                    store.writing_workspace_source(),
                ))
            })
            .map_err(Flow::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(crate) fn rollback_with_index<P>(
    store: &AppStore,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
    capture_index: impl FnOnce(&str) -> Result<FileIndex, Flow<Error, P>>,
) -> Result<StoredUndo, Flow<Error, P>> {
    let _content = store
        .content_read()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Storage))?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Storage))?;
    let saved = undos::read_in(&tx, original, operation)
        .map_err(|cause| Flow::original(Error::from(cause)))?
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    let file = saved
        .application
        .files
        .iter()
        .find(|f| f.path == path)
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    if file.target != ApplicationTarget::Disk {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    if file.phase == ApplicationPhase::Restored {
        return Ok(saved);
    }
    if !matches!(
        saved.application.status,
        ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
    ) || !matches!(
        file.phase,
        ApplicationPhase::Dispatched | ApplicationPhase::Acknowledged
    ) {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    let change = saved
        .changes
        .iter()
        .find(|c| c.path == path)
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;

    let index = match capture_index(path) {
        Ok(index) => Ok(index),
        Err(Flow::Fail(cause)) => Err(cause.error),
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Flow::original(Error::RecoveryRequired));
            }
            return Err(Flow::Need(input));
        }
    };
    undos::require_recovery_in(&tx, original, operation)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;

    let recover = (|| -> Result<StoredUndo, Error> {
        let restored = super::disk_restoration::restore(
            store,
            &db,
            original.manuscript,
            operation,
            change,
            index,
        )?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::Storage)?;
        restored.update_index(&tx, original.manuscript, path)?;
        undos::acknowledge_disk_restored_in(
            &tx,
            original,
            operation,
            path,
            restored.actual.as_ref(),
        )?;
        let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
        if saved.application.files.iter().all(|f| {
            matches!(
                f.phase,
                ApplicationPhase::Pending | ApplicationPhase::Restored
            )
        }) {
            undos::finish_rollback_in(&tx, original, operation)?;
        }
        let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
        tx.commit().map_err(|_| Error::RecoveryRequired)?;
        Ok(saved)
    })();
    recover.map_err(|cause| Flow::original(apply_disk::recovery_error(cause)))
}
