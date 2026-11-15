//! Original initial-copy transaction with a finite pause before any file creation.
use super::*;
use crate::agent::writing_workspace::WritingWorkspaceSnapshot;
use crate::ai::structured_resource_validation::Flow;
use std::path::Path;

pub(crate) enum Attempt<P> {
    Ready(Box<PreparedWorkspace>),
    Need(P),
}

pub(crate) fn prepare<P>(
    store: &AppStore,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
    manuscript_id: Uuid,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<Attempt<P>, Error> {
    let mut db = store.connection().map_err(|_| Error::Storage)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    check_binding(&tx, receipt.conversation_id, receipt.manuscript_id)?;
    let record = read(&tx, receipt.conversation_id, receipt.request_id)
        .map_err(WritingSubmissionError::from)?
        .ok_or(execution_workspaces::Error::Conflict)?;
    if record.context_snapshot_id != context_snapshot_id || manuscript_id != receipt.manuscript_id {
        return Err(execution_workspaces::Error::Conflict.into());
    }
    validate_scope(&record, receipt.manuscript_id)?;
    execution_workspaces::discard::ensure_preparation_allowed(
        &tx,
        receipt.conversation_id,
        receipt.request_id,
    )?;
    if execution_workspaces::read(&tx, receipt.conversation_id, receipt.request_id)?.is_some() {
        return Err(execution_workspaces::Error::Conflict.into());
    }
    let scope = WorkspaceIdentity {
        conversation_id: record.conversation_id,
        turn_id: record.turn_id,
        environment_id: format!("manuscript:{}", receipt.manuscript_id),
        lease_id: Uuid::new_v4(),
    };
    // Resolve the app-owned container (including macOS /var aliases),
    // then retain no-follow traversal for manuscript/source descendants.
    let manuscript_root = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Error::Storage)?;
    let snapshot = match capture(&tx, &manuscript_root, scope.clone()) {
        Ok(snapshot) => snapshot,
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Error::RecoveryRequired);
            }
            return Ok(Attempt::Need(input));
        }
        Err(Flow::Fail(failure)) => return Err(failure.error),
    };
    let copies = open(store)?;
    let lease = snapshot
        .create(&copies)
        .map_err(WritingSubmissionError::from)?;
    let binding = match execution_workspaces::prepare_in(
        &tx,
        receipt.request_id,
        receipt.manuscript_id,
        lease.manifest(),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            drop(lease);
            copies
                .discard(&scope)
                .map_err(|_| Error::RecoveryRequired)?;
            return Err(error.into());
        }
    };
    // On uncertain commit retain the ready copy for recovery; never delete
    // a lease that might have a committed database reference.
    tx.commit().map_err(|_| Error::RecoveryRequired)?;
    Ok(Attempt::Ready(Box::new(PreparedWorkspace {
        binding,
        lease,
    })))
}
