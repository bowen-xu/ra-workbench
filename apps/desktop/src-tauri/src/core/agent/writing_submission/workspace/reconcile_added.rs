//! Reconcile a dispatched disk change without replaying destination mutations.
use super::apply_disk::recovery_error;
use super::*;
use crate::agent::writing_workspace::index::FileIndex;
use crate::agent::{
    repository::{
        change_sets,
        proposal_applications::{
            self as applications, ApplicationPhase, ApplicationScope, ApplicationStatus,
            ApplicationTarget, StoredApplication,
        },
    },
    workspace::DiskTarget,
};
use crate::ai::structured_resource_validation::Flow;

impl WritingSubmissionService {
    /// Internal explicit recovery entry. Matching bytes establish current target
    /// state, not who wrote them; only a verified redundant staging link may be
    /// removed. The destination is never created, replaced or deleted here.
    pub async fn reconcile_added_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        self.reconcile_disk_file(scope, path, true).await
    }

    /// Existing-file recovery also requires the operation's original backup.
    /// Matching current bytes alone cannot prove a replacement/removal completed.
    pub async fn reconcile_disk_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        self.reconcile_disk_file(scope, path, false).await
    }

    async fn reconcile_disk_file(
        &self,
        scope: ApplicationScope,
        path: String,
        added_only: bool,
    ) -> Result<StoredApplication, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .reconcile_disk_file(
                    self.store.clone(),
                    scope,
                    path,
                    added_only,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            reconcile_with_index(&store, scope, &path, added_only, |_| {
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
    scope: ApplicationScope,
    path: &str,
    added_only: bool,
    capture_index: impl FnOnce(&str) -> Result<FileIndex, Flow<Error, P>>,
) -> Result<StoredApplication, Flow<Error, P>> {
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
    if file.target != ApplicationTarget::Disk || (added_only && change.before.is_some()) {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    if file.phase == ApplicationPhase::Acknowledged {
        return Ok(saved);
    }
    if file.phase != ApplicationPhase::Dispatched
        || !matches!(
            saved.status,
            ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
        )
    {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    // Resolve the pure policy before committed recovery work, but consume its
    // failure only at the original index check after destination proof.
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
    let source = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Flow::original(Error::Storage))?
        .join(scope.manuscript.to_string())
        .join("source");
    let target =
        DiskTarget::open(&source, path).map_err(|_| Flow::original(Error::RecoveryRequired))?;
    if let Some(before) = &change.before {
        target
            .reconcile_preserved(scope.operation, before)
            .map_err(|_| Flow::original(Error::RecoveryRequired))?;
    }
    let actual = if let Some(expected) = &change.after {
        Some(
            target
                .reconcile_create(scope.operation, expected)
                .map_err(|_| Flow::original(Error::RecoveryRequired))?,
        )
    } else {
        if change.before.is_none()
            || target
                .observe()
                .map_err(|_| Flow::original(Error::RecoveryRequired))?
                .is_some()
        {
            return Err(Flow::original(Error::RecoveryRequired));
        }
        None
    };
    // Keep the connection mutex across observation and confirmation.
    let index = index.map_err(|cause| Flow::original(recovery_error(cause)))?;
    apply_disk::confirm_disk(&index, &mut db, scope, path, change.before.as_ref(), actual)
        .map_err(|cause| Flow::original(recovery_error(cause)))
}
