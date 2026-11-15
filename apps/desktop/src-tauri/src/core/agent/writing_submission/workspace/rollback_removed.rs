//! Physical rollback of interrupted disk changes, not completed undo.
use super::*;
use crate::agent::repository::{
    change_sets,
    proposal_applications::{
        self as applications, ApplicationPhase, ApplicationScope, ApplicationStatus,
        ApplicationTarget, StoredApplication,
    },
};
use crate::{agent::writing_workspace::index::FileIndex, ai::structured_resource_validation::Flow};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RollbackKind {
    Added,
    Removed,
    Replaced,
}

impl WritingSubmissionService {
    /// Internal coordinator only. Caller owns live editor coverage and manuscript
    /// read/write coordination. A completed apply needs a separate undo operation.
    pub async fn rollback_removed_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        self.rollback_file(scope, path, RollbackKind::Removed).await
    }

    pub async fn rollback_added_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        self.rollback_file(scope, path, RollbackKind::Added).await
    }

    pub async fn rollback_replaced_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        self.rollback_file(scope, path, RollbackKind::Replaced)
            .await
    }

    async fn rollback_file(
        &self,
        scope: ApplicationScope,
        path: String,
        kind: RollbackKind,
    ) -> Result<StoredApplication, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .rollback_application_file(
                    self.store.clone(),
                    scope,
                    path,
                    kind,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            rollback_with_index(&store, scope, &path, kind, |_| {
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
    scope: ApplicationScope,
    path: &str,
    kind: RollbackKind,
    capture_index: impl FnOnce(&str) -> Result<FileIndex, Flow<Error, P>>,
) -> Result<StoredApplication, Flow<Error, P>> {
    let added = kind == RollbackKind::Added;
    let replaced = kind == RollbackKind::Replaced;
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Storage))?;
    let saved = applications::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    let file = saved
        .files
        .iter()
        .find(|file| file.path == path)
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    let proposal = change_sets::read_file_operation_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    let change = proposal
        .changes
        .iter()
        .find(|change| change.path == path)
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    if added {
        change.after.as_ref()
    } else {
        change.before.as_ref()
    }
    .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    if file.target != ApplicationTarget::Disk
        || (added && change.before.is_some())
        || (kind == RollbackKind::Removed && change.after.is_some())
        || (replaced && change.after.is_none())
    {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    if file.phase == ApplicationPhase::Restored {
        return Ok(saved);
    }
    if !matches!(
        saved.status,
        ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
    ) || !matches!(
        file.phase,
        ApplicationPhase::Dispatched | ApplicationPhase::Acknowledged
    ) {
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
    applications::require_recovery_in(&mut tx, scope)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;

    // All failures below leave the committed recovery barrier in place.
    let recover = (|| -> Result<StoredApplication, Error> {
        let restored = super::disk_restoration::restore(
            store,
            &db,
            scope.manuscript,
            scope.operation,
            change,
            index,
        )?;
        let mut tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::Storage)?;
        restored.update_index(&tx, scope.manuscript, path)?;
        applications::acknowledge_disk_restored_in(&tx, scope, path, restored.actual.as_ref())?;
        let state = applications::read_in(
            &tx,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )?;
        if state.files.iter().all(|file| {
            matches!(
                file.phase,
                ApplicationPhase::Pending | ApplicationPhase::Restored
            )
        }) {
            applications::finish_rollback_in(&mut tx, scope)?;
        }
        let result = applications::read_in(
            &tx,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )?;
        tx.commit().map_err(|_| Error::RecoveryRequired)?;
        Ok(result)
    })();
    recover.map_err(|cause| Flow::original(apply_disk::recovery_error(cause)))
}
