//! Disk proposal application. Editor transactions have a separate coordinator.
use super::*;
use crate::agent::repository::{
    change_sets,
    proposal_applications::{
        self as applications, ApplicationPhase, ApplicationScope, ApplicationTarget,
        StoredApplication,
    },
};
use crate::agent::writing_workspace::index::FileIndex;
use crate::ai::structured_resource_validation::Flow;

impl WritingSubmissionService {
    /// Coordinator entry, never model arguments. Application
    /// admission must already have preflighted the complete proposal.
    pub async fn apply_added_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        self.apply_disk_file(scope, path, true).await
    }

    /// Writing host only: caller has preflighted all files and owns live editor and
    /// manuscript read/write coordination. Never call for an active editor file.
    pub async fn apply_disk_proposal_file(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<StoredApplication, Error> {
        self.apply_disk_file(scope, path, false).await
    }

    async fn apply_disk_file(
        &self,
        scope: ApplicationScope,
        path: String,
        added_only: bool,
    ) -> Result<StoredApplication, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .apply_disk_file(
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
            apply_with_index(&store, scope, &path, added_only, |_| {
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

pub(crate) fn apply_with_index<P>(
    store: &AppStore,
    scope: ApplicationScope,
    path: &str,
    added_only: bool,
    capture_index: impl FnOnce(&str) -> Result<FileIndex, Flow<Error, P>>,
) -> Result<StoredApplication, Flow<Error, P>> {
    let _content = store
        .content_read()
        .map_err(|_| Flow::original(Error::Storage))?;
    // Keep the store's connection mutex across intent commit, disk I/O
    // and ACK, serializing other native file/index commands.
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Storage))?;
    let tx = db
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
    if file.target != ApplicationTarget::Disk {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    if file.phase != ApplicationPhase::Pending {
        // Never infer another dispatch from a repeated request.
        return Ok(saved);
    }
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
    if added_only && change.before.is_some() {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    let before = change.before.clone();
    let mutation = match super::disk_mutation::prepare_with_index(
        store,
        &tx,
        scope.manuscript,
        change,
        capture_index,
    ) {
        Ok(mutation) => mutation,
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Flow::original(Error::RecoveryRequired));
            }
            return Err(Flow::Need(input));
        }
        Err(failure) => return Err(failure),
    };
    if applications::dispatch_in(&tx, scope, path)
        .map_err(|cause| Flow::original(Error::from(cause)))?
        != applications::DispatchAdmission::NewIntent
    {
        return Err(Flow::original(Error::RecoveryRequired));
    }
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;
    // Any subsequent failure keeps the committed intent. It must not
    // become a new operation, nor remove a possibly published file.
    let applied = (|| -> Result<StoredApplication, Error> {
        let index = mutation.index();
        let actual = mutation.execute(scope.operation)?;
        confirm_disk(&index, &mut db, scope, path, before.as_ref(), actual)
    })();
    if let Err(cause) = applied {
        if let Ok(mut tx) = db.transaction_with_behavior(TransactionBehavior::Immediate)
            && applications::require_recovery_in(&mut tx, scope).is_ok()
        {
            let _ = tx.commit();
        }
        return Err(Flow::original(recovery_error(cause)));
    }
    applied.map_err(Flow::original)
}

/// Same transaction for disk application and explicitly requested reconciliation.
/// Caller holds the connection mutex and supplies a freshly observed version.
pub(super) fn confirm_disk(
    index: &FileIndex,
    db: &mut rusqlite::Connection,
    scope: ApplicationScope,
    path: &str,
    before: Option<&research_agent_core::workspace::FileVersion>,
    actual: Option<research_agent_core::workspace::FileVersion>,
) -> Result<StoredApplication, Error> {
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    super::disk_mutation::update_index_with_source(
        index,
        &tx,
        scope.manuscript,
        path,
        before,
        actual.as_ref(),
    )?;
    applications::acknowledge_in(
        &tx,
        scope,
        &applications::FileReceipt {
            path: path.into(),
            after: actual,
            editor: None,
        },
    )?;
    let saved = applications::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )?;
    if saved
        .files
        .iter()
        .all(|file| file.phase == ApplicationPhase::Acknowledged)
    {
        applications::finish_in(&mut tx, scope)?;
    }
    let saved = applications::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )?;
    tx.commit().map_err(|_| Error::RecoveryRequired)?;
    Ok(saved)
}

pub(super) fn recovery_error(cause: Error) -> Error {
    match cause {
        Error::Submission(WritingSubmissionError::Native(mut cause)) => {
            cause.0.save_outcome =
                Some(crate::runtime_plugins::literature_catalog::save::SaveOutcome::Unknown);
            WritingSubmissionError::Native(cause).into()
        }
        _ => Error::RecoveryRequired,
    }
}
