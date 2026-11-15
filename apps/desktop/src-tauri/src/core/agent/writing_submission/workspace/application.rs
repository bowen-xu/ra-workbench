//! Admission records intent before any application side effect.
use super::*;
use crate::agent::repository::proposal_applications::{self, ApplicationAdmission};
use crate::agent::writing_workspace::WritingWorkspaceSnapshot;
use crate::ai::structured_resource_validation::Flow;
use std::path::Path;

impl WritingSubmissionService {
    pub async fn find_proposal_application(
        &self,
        conversation: Uuid,
        manuscript: Uuid,
        proposal: Uuid,
    ) -> Result<Option<proposal_applications::StoredApplication>, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            proposal_applications::find_in(&tx, conversation, manuscript, proposal)
                .map_err(Into::into)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn read_proposal_application(
        &self,
        conversation: Uuid,
        manuscript: Uuid,
        proposal: Uuid,
        operation: Uuid,
    ) -> Result<proposal_applications::StoredApplication, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            proposal_applications::read_in(&tx, conversation, manuscript, proposal, operation)
                .map_err(Into::into)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn begin_proposal_application(
        &self,
        conversation: Uuid,
        proposal: Uuid,
        operation: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<ApplicationAdmission, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .begin_application(
                    self.store.clone(),
                    conversation,
                    proposal,
                    operation,
                    request,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            begin_with_snapshot(
                &store,
                conversation,
                proposal,
                operation,
                request.manuscript_id,
                |db, root, identity| {
                    preflight::static_snapshot(&store, db, root, identity, request)
                },
            )
            .map_err(Flow::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Reopen an interrupted application only after a fresh source/editor
    /// preflight. This is deliberately separate from `begin`: its operation
    /// identity already exists and only all-pending journals are eligible.
    pub async fn resume_proposal_application(
        &self,
        scope: proposal_applications::ApplicationScope,
        request: WritingWorkspaceRequest,
    ) -> Result<proposal_applications::StoredApplication, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .resume_application(
                    self.store.clone(),
                    scope,
                    request,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            resume_with_snapshot(
                &store,
                scope,
                request.manuscript_id,
                |db, root, identity| {
                    preflight::static_snapshot(&store, db, root, identity, request)
                },
            )
            .map_err(Flow::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(crate) fn begin_with_snapshot<P>(
    store: &AppStore,
    conversation: Uuid,
    proposal: Uuid,
    operation: Uuid,
    manuscript: Uuid,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<ApplicationAdmission, Flow<Error, P>> {
    let _content = store
        .content_read()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Storage))?;
    if proposal_applications::exists_in(&tx, conversation, manuscript, proposal, operation)
        .map_err(|cause| Flow::original(Error::from(cause)))?
    {
        return Ok(ApplicationAdmission::Existing(operation));
    }
    let checked = match preflight::preflight_application_in_with_snapshot(
        store,
        &tx,
        conversation,
        proposal,
        manuscript,
        capture,
    ) {
        Ok(checked) => checked,
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Flow::original(Error::RecoveryRequired));
            }
            return Err(Flow::Need(input));
        }
        Err(failure) => return Err(failure),
    };
    let admitted = proposal_applications::begin_with_targets_in(
        &mut tx,
        conversation,
        manuscript,
        proposal,
        operation,
        &checked.targets,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    // Uncertain commit keeps the exact operation identity for retry/read.
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;
    Ok(admitted)
}

pub(crate) fn resume_with_snapshot<P>(
    store: &AppStore,
    scope: proposal_applications::ApplicationScope,
    manuscript: Uuid,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<WritingWorkspaceSnapshot, Flow<Error, P>>,
) -> Result<proposal_applications::StoredApplication, Flow<Error, P>> {
    let _content = store
        .content_read()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Storage))?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Storage))?;
    if manuscript != scope.manuscript {
        return Err(Flow::original(execution_workspaces::Error::Conflict.into()));
    }
    let saved = proposal_applications::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    if saved
        .files
        .iter()
        .any(|file| file.phase != proposal_applications::ApplicationPhase::Pending)
    {
        return Err(Flow::original(execution_workspaces::Error::Conflict.into()));
    }
    // Validate the live disk/editor baseline before reopening dispatch.
    // This consumes the request only after the full check succeeds.
    let checked = match preflight::preflight_resume_in_with_snapshot(
        store,
        &tx,
        scope.conversation,
        scope.proposal,
        manuscript,
        capture,
    ) {
        Ok(checked) => checked,
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Flow::original(Error::RecoveryRequired));
            }
            return Err(Flow::Need(input));
        }
        Err(failure) => return Err(failure),
    };
    proposal_applications::rebind_pending_targets_in(&mut tx, scope, &checked.targets)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    proposal_applications::resume_in(&mut tx, scope)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    let resumed = proposal_applications::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;
    Ok(resumed)
}
