//! Execute an inverse disk plan using its own operation identity and journal.
use super::*;
use crate::agent::repository::{
    proposal_applications::{
        self as applications, ApplicationPhase, ApplicationScope, ApplicationTarget,
    },
    proposal_undos::{self as undos, StoredUndo},
};
use crate::agent::writing_workspace::index::FileIndex;
use crate::ai::structured_resource_validation::Flow;
impl WritingSubmissionService {
    /// Host owns the complete manuscript lease; active buffers use editor undo.
    pub async fn apply_disk_undo_file(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        path: String,
    ) -> Result<StoredUndo, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .apply_undo_file(
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
            apply_with_index(&store, original, operation, &path, |_| {
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
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
    capture_index: impl FnOnce(&str) -> Result<FileIndex, Flow<Error, P>>,
) -> Result<StoredUndo, Flow<Error, P>> {
    let _content = store
        .content_read()
        .map_err(|_| Flow::original(Error::Storage))?;
    // Keep the same connection mutex across intent, disk I/O and ACK.
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
    if file.target != ApplicationTarget::Disk {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    if file.phase != ApplicationPhase::Pending {
        return Ok(saved);
    }
    let change = saved
        .changes
        .iter()
        .find(|c| c.path == path)
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    let before = change.before.clone();
    let mutation = match super::disk_mutation::prepare_with_index(
        store,
        &tx,
        original.manuscript,
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
        Err(cause) => return Err(cause),
    };
    if undos::dispatch_in(&tx, original, operation, path)
        .map_err(|cause| Flow::original(Error::from(cause)))?
        != applications::DispatchAdmission::NewIntent
    {
        return Err(Flow::original(Error::RecoveryRequired));
    }
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;
    let applied = (|| -> Result<StoredUndo, Error> {
        let index = mutation.index();
        let actual = mutation.execute(operation)?;
        confirm_disk(
            &index,
            &mut db,
            original,
            operation,
            path,
            before.as_ref(),
            actual,
        )
    })();
    if let Err(cause) = applied {
        if let Ok(tx) = db.transaction_with_behavior(TransactionBehavior::Immediate)
            && undos::require_recovery_in(&tx, original, operation).is_ok()
        {
            let _ = tx.commit();
        }
        return Err(Flow::original(apply_disk::recovery_error(cause)));
    }
    applied.map_err(Flow::original)
}

/// Same transaction for disk application and explicitly requested reconciliation.
/// Caller holds the connection mutex and supplies a freshly observed version.
pub(super) fn confirm_disk(
    index: &FileIndex,
    db: &mut rusqlite::Connection,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
    before: Option<&research_agent_core::workspace::FileVersion>,
    actual: Option<research_agent_core::workspace::FileVersion>,
) -> Result<StoredUndo, Error> {
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    super::disk_mutation::update_index_with_source(
        index,
        &tx,
        original.manuscript,
        path,
        before,
        actual.as_ref(),
    )?;
    undos::acknowledge_in(
        &tx,
        original,
        operation,
        &applications::FileReceipt {
            path: path.into(),
            after: actual,
            editor: None,
        },
    )?;
    let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
    if saved
        .application
        .files
        .iter()
        .all(|file| file.phase == ApplicationPhase::Acknowledged)
    {
        undos::finish_in(&mut tx, original, operation)?;
    }
    let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
    tx.commit().map_err(|_| Error::RecoveryRequired)?;
    Ok(saved)
}
