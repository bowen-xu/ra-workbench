//! Native writing execution-copy preparation. No model request or execution grant.
use super::*;
use crate::agent::writing_workspace::WritingWorkspaceRequest;
use crate::agent::{
    repository::execution_workspaces::{self, Binding},
    workspace::{LeaseError, WorkspaceLease, WorkspaceStore},
};
use research_agent_core::workspace::WorkspaceIdentity;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Submission(#[from] WritingSubmissionError),
    #[error(transparent)]
    Binding(#[from] execution_workspaces::Error),
    #[error(transparent)]
    Lease(#[from] LeaseError),
    #[error("execution workspace storage unavailable")]
    Storage,
    #[error("execution workspace has not been prepared")]
    NotFound,
    #[error("execution workspace requires recovery")]
    RecoveryRequired,
}
pub struct PreparedWorkspace {
    pub binding: Binding,
    pub lease: WorkspaceLease,
}
fn open(store: &AppStore) -> Result<WorkspaceStore, Error> {
    let path = store.agent_workspace_directory();
    std::fs::create_dir_all(path).map_err(|_| Error::Storage)?;
    if std::fs::symlink_metadata(path)
        .map_err(|_| Error::Storage)?
        .file_type()
        .is_symlink()
    {
        return Err(Error::Storage);
    }
    let canonical = path.canonicalize().map_err(|_| Error::Storage)?;
    // AppStore chooses this private root; renderer/model paths are not accepted.
    Ok(WorkspaceStore::open(&canonical, |_| Ok(()))?)
}
impl WritingSubmissionService {
    /// Recover a prepared but not yet approved successor without recreating it.
    pub async fn restore_prepared_next_workspace(
        &self,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
        generation: u32,
        predecessor: Uuid,
        lease_id: Uuid,
    ) -> Result<PreparedWorkspace, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (binding, manifest) = {
                let db = store.connection().map_err(|_| Error::Storage)?;
                check_binding(&db, receipt.conversation_id, receipt.manuscript_id)?;
                let (binding, manifest) = execution_workspaces::generation::read_prepared(
                    &db,
                    receipt.conversation_id,
                    receipt.request_id,
                    generation,
                    predecessor,
                    lease_id,
                )?;
                if binding.manuscript_id != receipt.manuscript_id
                    || binding.context_snapshot_id != context_snapshot_id
                {
                    return Err(execution_workspaces::Error::Conflict.into());
                }
                (binding, manifest)
            };
            let lease = open(&store)?
                .restore(&manifest.identity)
                .map_err(|_| Error::RecoveryRequired)?;
            if lease.manifest() != &manifest
                || !lease
                    .changes()
                    .map_err(|_| Error::RecoveryRequired)?
                    .is_empty()
            {
                return Err(Error::RecoveryRequired);
            }
            let db = store.connection().map_err(|_| Error::Storage)?;
            let (checked, _) = execution_workspaces::generation::read_prepared(
                &db,
                receipt.conversation_id,
                receipt.request_id,
                generation,
                predecessor,
                lease_id,
            )?;
            if checked != binding {
                return Err(execution_workspaces::Error::Conflict.into());
            }
            Ok(PreparedWorkspace { binding, lease })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Reacquire the exact approved live successor. Never create a missing copy,
    /// and recheck its grant after acquiring the filesystem owner.
    pub async fn restore_next_workspace(
        &self,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
        generation: u32,
        predecessor: Uuid,
        lease_id: Uuid,
    ) -> Result<PreparedWorkspace, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (binding, manifest) = {
                let db = store.connection().map_err(|_| Error::Storage)?;
                check_binding(&db, receipt.conversation_id, receipt.manuscript_id)?;
                let (binding, manifest) = execution_workspaces::generation::read_approved(
                    &db,
                    receipt.conversation_id,
                    receipt.request_id,
                    generation,
                    predecessor,
                    lease_id,
                )?;
                if binding.manuscript_id != receipt.manuscript_id
                    || binding.context_snapshot_id != context_snapshot_id
                {
                    return Err(execution_workspaces::Error::Conflict.into());
                }
                (binding, manifest)
            };
            let lease = open(&store)?.restore(&manifest.identity)?;
            if lease.manifest() != &manifest {
                return Err(execution_workspaces::Error::Conflict.into());
            }
            let db = store.connection().map_err(|_| Error::Storage)?;
            execution_workspaces::generation::authorize(
                &db,
                &binding,
                generation,
                predecessor,
                &manifest,
            )?;
            Ok(PreparedWorkspace { binding, lease })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Recover only the original ready copy for an unfinished preparation. It
    /// never recreates missing bytes or grants permission to run commands.
    pub async fn recover_next_workspace(
        &self,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
        generation: u32,
        predecessor: Uuid,
        lease_id: Uuid,
    ) -> Result<PreparedWorkspace, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (binding, manifest) = consent::consent_transaction(&store, |tx| {
                check_binding(tx, receipt.conversation_id, receipt.manuscript_id)?;
                let (binding, manifest) = execution_workspaces::generation::read_preparing(
                    tx,
                    receipt.conversation_id,
                    receipt.request_id,
                    generation,
                    predecessor,
                    lease_id,
                )?;
                if binding.manuscript_id != receipt.manuscript_id
                    || binding.context_snapshot_id != context_snapshot_id
                {
                    return Err(execution_workspaces::Error::Conflict.into());
                }
                let checked = execution_workspaces::generation::prepare_in(
                    tx,
                    receipt.request_id,
                    receipt.manuscript_id,
                    generation,
                    predecessor,
                    &manifest,
                )?;
                if checked != binding {
                    return Err(execution_workspaces::Error::Conflict.into());
                }
                Ok((binding, manifest))
            })?;
            let lease = open(&store)
                .map_err(|_| Error::RecoveryRequired)?
                .restore(&manifest.identity)
                .map_err(|_| Error::RecoveryRequired)?;
            if lease.manifest() != &manifest
                || !lease
                    .changes()
                    .map_err(|_| Error::RecoveryRequired)?
                    .is_empty()
            {
                return Err(Error::RecoveryRequired);
            }
            consent::consent_transaction(&store, |tx| {
                execution_workspaces::generation::finish_in(tx, generation, predecessor, &binding)?;
                Ok(())
            })
            .map_err(|_| Error::RecoveryRequired)?;
            Ok(PreparedWorkspace { binding, lease })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Prepare a later copy from a fresh native snapshot. The caller supplies an
    /// operation-owned lease ID and complete editor-buffer coverage. The durable
    /// intent precedes filesystem creation; uncertain attempts need recovery.
    pub async fn prepare_next_workspace(
        &self,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
        generation: u32,
        predecessor: Uuid,
        lease_id: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<PreparedWorkspace, Error> {
        let successor = successor_preparation::Successor {
            generation,
            predecessor,
            lease_id,
        };
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .prepare_next_workspace(
                    self.store.clone(),
                    receipt,
                    context_snapshot_id,
                    successor,
                    request,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            match successor_preparation::prepare(
                &store,
                receipt,
                context_snapshot_id,
                successor,
                request.manuscript_id,
                |db, root, scope| {
                    store
                        .snapshot_writing_workspace_in(db, root, scope, request, |_, _| Ok(()))
                        .map_err(|cause| {
                            crate::ai::structured_resource_validation::Flow::<
                                Error,
                                std::convert::Infallible,
                            >::original(
                                WritingSubmissionError::from(cause).into()
                            )
                        })
                },
            )? {
                preparation::Attempt::Ready(prepared) => Ok(*prepared),
                preparation::Attempt::Need(never) => match never {},
            }
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Caller must supply complete coverage of unsaved editor buffers. The exact
    /// context identity is confirmed before filesystem reads. Retrying preparation
    /// never replaces an existing copy: recover it explicitly instead.
    pub async fn prepare_workspace(
        &self,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<PreparedWorkspace, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .prepare_workspace(
                    self.store.clone(),
                    receipt,
                    context_snapshot_id,
                    request,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            match preparation::prepare(
                &store,
                receipt,
                context_snapshot_id,
                request.manuscript_id,
                |db, root, scope| {
                    store
                        .snapshot_writing_workspace_in(db, root, scope, request, |_, _| Ok(()))
                        .map_err(|cause| {
                            crate::ai::structured_resource_validation::Flow::<
                                Error,
                                std::convert::Infallible,
                            >::original(
                                WritingSubmissionError::from(cause).into()
                            )
                        })
                },
            )? {
                preparation::Attempt::Ready(prepared) => Ok(*prepared),
                preparation::Attempt::Need(never) => match never {},
            }
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Restore frozen bytes even if the original manuscript has since changed.
    /// Active or uncertain execution markers are rejected by WorkspaceStore.
    pub async fn restore_workspace(
        &self,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
    ) -> Result<PreparedWorkspace, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let db = store.connection().map_err(|_| Error::Storage)?;
            check_binding(&db, receipt.conversation_id, receipt.manuscript_id)?;
            let (binding, manifest) =
                execution_workspaces::read(&db, receipt.conversation_id, receipt.request_id)?
                    .ok_or(Error::NotFound)?;
            if binding.manuscript_id != receipt.manuscript_id
                || binding.context_snapshot_id != context_snapshot_id
            {
                return Err(execution_workspaces::Error::Conflict.into());
            }
            let lease = open(&store)?.restore(&manifest.identity)?;
            if lease.manifest() != &manifest {
                return Err(execution_workspaces::Error::Conflict.into());
            }
            Ok(PreparedWorkspace { binding, lease })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

#[path = "workspace/consent.rs"]
mod consent;
#[path = "workspace/discard.rs"]
pub(crate) mod discard;
#[path = "workspace/review.rs"]
mod review;
pub use review::WorkspaceReview;
#[cfg(test)]
#[path = "workspace/tests.rs"]
mod tests;

#[path = "workspace/object_staging.rs"]
mod object_staging;
#[path = "workspace/publication.rs"]
mod publication;
pub(crate) use publication::schedule_object_recovery;
#[path = "workspace/application.rs"]
pub(crate) mod application;
#[path = "workspace/application_checkpoint.rs"]
mod application_checkpoint;
#[path = "workspace/application_rollback_check.rs"]
pub(crate) mod application_rollback_check;
#[path = "workspace/apply_disk.rs"]
pub(crate) mod apply_disk;
#[path = "workspace/apply_editor.rs"]
mod apply_editor;
#[path = "workspace/restore_editor.rs"]
pub(crate) mod restore_editor;
pub use apply_editor::{EditorAdmission, EditorDispatch, EditorObservation};
pub use restore_editor::EditorRestoreAdmission;
#[path = "workspace/disk_mutation.rs"]
mod disk_mutation;
#[path = "workspace/disk_restoration.rs"]
mod disk_restoration;
#[path = "workspace/frozen_review.rs"]
mod frozen_review;
#[path = "workspace/preflight.rs"]
mod preflight;
#[path = "workspace/reconcile_added.rs"]
pub(crate) mod reconcile_added;
#[path = "workspace/rollback_plan.rs"]
mod rollback_plan;
#[path = "workspace/rollback_removed.rs"]
pub(crate) mod rollback_removed;
#[path = "workspace/undo_checkpoint.rs"]
pub(crate) mod undo_checkpoint;
#[path = "workspace/undo_disk.rs"]
pub(crate) mod undo_disk;
#[path = "workspace/undo_editor.rs"]
mod undo_editor;
#[path = "workspace/undo_preparation.rs"]
mod undo_preparation;
#[path = "workspace/undo_preview.rs"]
pub(crate) mod undo_preview;
#[path = "workspace/undo_reconcile.rs"]
pub(crate) mod undo_reconcile;
#[path = "workspace/undo_restore_editor.rs"]
pub(crate) mod undo_restore_editor;
#[path = "workspace/undo_resume.rs"]
pub(crate) mod undo_resume;
#[path = "workspace/undo_rollback.rs"]
pub(crate) mod undo_rollback;
#[path = "workspace/undo_rollback_plan.rs"]
pub(crate) mod undo_rollback_plan;
pub(crate) use undo_checkpoint::UndoCheckpointVersion;
pub use undo_editor::UndoEditorAdmission;

#[path = "workspace/preparation.rs"]
pub(crate) mod preparation;

#[path = "workspace/successor_preparation.rs"]
pub(crate) mod successor_preparation;

#[path = "workspace/undo_admission.rs"]
pub(crate) mod undo_admission;

#[path = "workspace/restoration_source.rs"]
pub(crate) mod restoration_source;
