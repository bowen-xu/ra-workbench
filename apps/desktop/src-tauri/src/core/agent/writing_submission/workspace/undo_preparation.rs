//! Freeze a prepared undo exactly once. No file effects or editor dispatch.
use super::*;
use crate::agent::repository::{
    proposal_applications::ApplicationScope,
    proposal_undos::{self, StoredUndo},
};
use crate::ai::structured_resource_validation::Flow;
use std::convert::Infallible;

impl WritingSubmissionService {
    /// Discover a durable operation after losing frontend state. Never resumes it.
    pub async fn find_latest_proposal_undo(
        &self,
        original: ApplicationScope,
    ) -> Result<Option<StoredUndo>, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            proposal_undos::find_latest_in(&tx, original).map_err(Error::from)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Release an unused baseline without reading or writing the current files.
    pub async fn cancel_pending_proposal_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
    ) -> Result<StoredUndo, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            proposal_undos::cancel_pending_in(&tx, original, operation)?;
            let saved =
                proposal_undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok(saved)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// A lost host/receipt stops further dispatch without inventing an outcome.
    pub async fn defer_proposal_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
    ) -> Result<StoredUndo, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            let saved =
                proposal_undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            if saved.application.status
                != crate::agent::repository::proposal_applications::ApplicationStatus::Applying
            {
                return Ok(saved);
            }
            proposal_undos::require_recovery_in(&tx, original, operation)?;
            let saved =
                proposal_undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok(saved)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Enter execution only after the entire frozen undo still matches reality.
    /// A repeated start is a state read, never a restart of an uncertain operation.
    pub async fn start_proposal_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<StoredUndo, Error> {
        if request.manuscript_id != original.manuscript {
            return Err(LeaseError::Conflict.into());
        }
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .admit_undo(
                    self.store.clone(),
                    request,
                    undo_admission::Admission {
                        original,
                        operation,
                        kind: undo_admission::Kind::Start,
                        checkpoints: vec![],
                    },
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            undo_admission::admit(
                &store,
                original,
                operation,
                undo_admission::Kind::Start,
                |db, root, identity| {
                    let checkpoints = undo_checkpoint::read_in(
                        db,
                        store.writing_checkpoint_source(),
                        &request,
                        &[],
                    )
                    .map_err(Flow::original)?;
                    let snapshot = preflight::static_snapshot(&store, db, root, identity, request)?;
                    Ok((snapshot, checkpoints))
                },
            )
            .map_err(Flow::<Error, Infallible>::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Complete buffer coverage and the manuscript lease belong to the caller.
    /// Existing operations retain their original baseline, even after later edits.
    pub async fn prepare_proposal_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<StoredUndo, Error> {
        if request.manuscript_id != original.manuscript {
            return Err(LeaseError::Conflict.into());
        }
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .admit_undo(
                    self.store.clone(),
                    request,
                    undo_admission::Admission {
                        original,
                        operation,
                        kind: undo_admission::Kind::Prepare,
                        checkpoints: vec![],
                    },
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            undo_admission::admit(
                &store,
                original,
                operation,
                undo_admission::Kind::Prepare,
                |db, root, identity| {
                    let checkpoints = undo_checkpoint::read_in(
                        db,
                        store.writing_checkpoint_source(),
                        &request,
                        &[],
                    )
                    .map_err(Flow::original)?;
                    let snapshot = preflight::static_snapshot(&store, db, root, identity, request)?;
                    Ok((snapshot, checkpoints))
                },
            )
            .map_err(Flow::<Error, Infallible>::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Read only the frozen operation, including after losing the original view.
    pub async fn read_proposal_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
    ) -> Result<Option<StoredUndo>, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            proposal_undos::read_in(&tx, original, operation).map_err(Error::from)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}
