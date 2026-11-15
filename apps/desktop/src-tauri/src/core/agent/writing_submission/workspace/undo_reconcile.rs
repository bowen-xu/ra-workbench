//! Explicitly reconcile an inverse disk effect; never replay its write.
use super::*;
use crate::agent::writing_workspace::index::FileIndex;
use crate::agent::{
    repository::{
        proposal_applications::{
            ApplicationPhase, ApplicationScope, ApplicationStatus, ApplicationTarget,
        },
        proposal_undos::{self as undos, StoredUndo},
    },
    workspace::DiskTarget,
};
use crate::ai::structured_resource_validation::Flow;
impl WritingSubmissionService {
    pub async fn reconcile_disk_undo_file(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        path: String,
    ) -> Result<StoredUndo, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .reconcile_undo_file(
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
            reconcile_with_index(&store, original, operation, &path, |_| {
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

pub(crate) fn reconcile_with_index<P>(
    store: &AppStore,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
    capture_index: impl FnOnce(&str) -> Result<FileIndex, Flow<Error, P>>,
) -> Result<StoredUndo, Flow<Error, P>> {
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
        .find(|file| file.path == path)
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    let change = saved
        .changes
        .iter()
        .find(|c| c.path == path)
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    if file.target != ApplicationTarget::Disk {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    if file.phase == ApplicationPhase::Acknowledged {
        return Ok(saved);
    }
    if file.phase != ApplicationPhase::Dispatched
        || !matches!(
            saved.application.status,
            ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
        )
    {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
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
    (|| -> Result<StoredUndo, Error> {
        let source = store
            .manuscript_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?
            .join(original.manuscript.to_string())
            .join("source");
        let target = DiskTarget::open(&source, path).map_err(|_| Error::RecoveryRequired)?;
        if let Some(before) = &change.before {
            target
                .reconcile_preserved(operation, before)
                .map_err(|_| Error::RecoveryRequired)?;
        }
        let actual = if let Some(expected) = &change.after {
            Some(
                target
                    .reconcile_create(operation, expected)
                    .map_err(|_| Error::RecoveryRequired)?,
            )
        } else {
            if change.before.is_none()
                || target
                    .observe()
                    .map_err(|_| Error::RecoveryRequired)?
                    .is_some()
            {
                return Err(Error::RecoveryRequired);
            }
            None
        };
        // Keep the connection mutex across observation and confirmation.
        let index = index.map_err(apply_disk::recovery_error)?;
        super::undo_disk::confirm_disk(
            &index,
            &mut db,
            original,
            operation,
            path,
            change.before.as_ref(),
            actual,
        )
        .map_err(apply_disk::recovery_error)
    })()
    .map_err(Flow::original)
}
