//! Full-plan validation before editor compensation and before remaining disk effects.
use super::*;
use crate::agent::{
    repository::{
        proposal_applications::{
            ApplicationPhase, ApplicationScope, ApplicationStatus, ApplicationTarget,
        },
        proposal_undos::{self as undos, StoredUndo},
    },
    workspace::{DiskTarget, ProposalObjects},
};
use crate::{
    agent::writing_submission::workspace::restoration_source::RestorationSource,
    ai::structured_resource_validation::Flow,
};

impl WritingSubmissionService {
    /// Host holds complete live-buffer coverage and the manuscript lease until
    /// this finishes. Every editor must already be restored or never dispatched.
    pub async fn rollback_disk_proposal_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<StoredUndo, Error> {
        self.rollback_disk_proposal_undo_with_checkpoints(original, operation, request, vec![])
            .await
    }
    pub(crate) async fn rollback_disk_proposal_undo_with_checkpoints(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
        checkpoints: Vec<UndoCheckpointVersion>,
    ) -> Result<StoredUndo, Error> {
        let saved = self
            .check_undo_rollback(original, operation, request, false, checkpoints)
            .await?;
        if saved.application.status == ApplicationStatus::RolledBack {
            return Ok(saved);
        }
        let mut latest = saved.application.clone();
        for file in saved.application.files.iter().rev() {
            if matches!(
                file.phase,
                ApplicationPhase::Pending | ApplicationPhase::Restored
            ) {
                continue;
            }
            latest = self
                .rollback_disk_undo_file(original, operation, file.path.clone())
                .await?
                .application;
        }
        if latest.status != ApplicationStatus::RolledBack {
            return Err(Error::RecoveryRequired);
        }
        self.read_proposal_undo(original, operation)
            .await?
            .ok_or(LeaseError::Conflict.into())
    }
    /// Validate every target before the host dispatches editor compensation.
    pub async fn prepare_proposal_undo_rollback(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<StoredUndo, Error> {
        self.prepare_proposal_undo_rollback_with_checkpoints(original, operation, request, vec![])
            .await
    }
    pub(crate) async fn prepare_proposal_undo_rollback_with_checkpoints(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
        checkpoints: Vec<UndoCheckpointVersion>,
    ) -> Result<StoredUndo, Error> {
        self.check_undo_rollback(original, operation, request, true, checkpoints)
            .await
    }
    async fn check_undo_rollback(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
        allow_editor_effects: bool,
        checkpoint_versions: Vec<UndoCheckpointVersion>,
    ) -> Result<StoredUndo, Error> {
        if request.manuscript_id != original.manuscript {
            return Err(LeaseError::Conflict.into());
        }
        let input = UndoRollbackCheck {
            original,
            operation,
            allow_editor_effects,
            checkpoints: checkpoint_versions,
        };
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .check_undo_compensation(
                    self.store.clone(),
                    request,
                    input,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            check_with_source(
                &store,
                &request,
                &input,
                &mut store.writing_workspace_source(),
            )
            .map_err(Flow::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(crate) fn check_with_source<S: RestorationSource>(
    store: &AppStore,
    request: &WritingWorkspaceRequest,
    input: &UndoRollbackCheck,
    source_reader: &mut S,
) -> Result<StoredUndo, Flow<Error, S::Pause>> {
    let original = input.original;
    let operation = input.operation;
    let allow_editor_effects = input.allow_editor_effects;
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
    if saved.application.status == ApplicationStatus::RolledBack {
        return Ok(saved);
    }
    if !matches!(
        saved.application.status,
        ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
    ) || saved.application.files.iter().any(|f| {
        f.phase == ApplicationPhase::Conflict
            || (f.target == ApplicationTarget::Disk
                && request.buffers.iter().any(|b| b.path == f.path))
    }) {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    let checkpoints =
        match undo_checkpoint::read_with(request, &input.checkpoints, |manuscript, path| {
            source_reader.checkpoint(&tx, manuscript, path)
        }) {
            Ok(value) => value,
            Err(Flow::Need(input)) => {
                if tx.rollback().is_err() || !db.is_autocommit() {
                    db.require_recovery();
                    return Err(Flow::original(Error::RecoveryRequired));
                }
                return Err(Flow::Need(input));
            }
            Err(cause) => return Err(cause),
        };
    let proposal = crate::agent::repository::change_sets::read_file_operation_in(
        &tx,
        original.conversation,
        original.manuscript,
        original.proposal,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    let source = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Flow::original(Error::Storage))?
        .join(original.manuscript.to_string())
        .join("source");
    let objects = ProposalObjects::open(
        &store
            .content_directory()
            .canonicalize()
            .map_err(|_| Flow::original(Error::Storage))?,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    for (file, change) in saved.application.files.iter().zip(&saved.changes) {
        for version in [&change.before, &change.after].into_iter().flatten() {
            let relative: String = tx
                .query_row(
                    "select relative_path from content_objects where sha256=?1",
                    [&version.sha256],
                    |r| r.get(0),
                )
                .map_err(|_| Flow::original(Error::Storage))?;
            objects
                .verify_registered(version, &relative)
                .map_err(|cause| Flow::original(Error::from(cause)))?;
        }
        if file.path != change.path {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
        let target = DiskTarget::open(&source, &file.path)
            .map_err(|_| Flow::original(Error::RecoveryRequired))?;
        let actual = target
            .observe()
            .map_err(|_| Flow::original(Error::RecoveryRequired))?;
        if let ApplicationTarget::Editor {
            expected,
            result_revision,
        } = &file.target
        {
            let buffer = request
                .buffers
                .iter()
                .find(|b| b.path == file.path)
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            if request
                .buffers
                .iter()
                .filter(|b| b.path == file.path)
                .count()
                != 1
            {
                return Err(Flow::original(LeaseError::Conflict.into()));
            }
            let restoration =
                undos::read_editor_restoration_in(&tx, original, operation, &file.path)
                    .map_err(|cause| Flow::original(Error::from(cause)))?;
            let owner = restoration
                .as_ref()
                .map(|r| r.expected.session_id)
                .unwrap_or(expected.session_id);
            let (version, revision) = match file.phase {
                ApplicationPhase::Pending if restoration.is_none() => {
                    (&change.before, expected.revision)
                }
                ApplicationPhase::Acknowledged if allow_editor_effects && restoration.is_none() => {
                    (
                        &change.after,
                        result_revision
                            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?,
                    )
                }
                ApplicationPhase::Restored => (
                    &change.before,
                    restoration
                        .as_ref()
                        .and_then(|r| r.result_revision)
                        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?,
                ),
                _ => return Err(Flow::original(LeaseError::Conflict.into())),
            };
            let version = version
                .as_ref()
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            let disk = actual
                .as_ref()
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            let recorded_source = change
                .source
                .as_ref()
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            // A confirmed editor Undo may have been formally saved before
            // recovery. After editor restoration, that same saved inverse
            // result may remain on disk until restoration is saved. Admit
            // only that frozen version; live editor receipts and the file
            // index must still match independently below.
            let saved_inverse = matches!(
                file.phase,
                ApplicationPhase::Acknowledged | ApplicationPhase::Restored
            ) && change.after.as_ref() == Some(disk);
            if buffer.editor.session_id == owner {
                if buffer.editor.revision != revision {
                    return Err(Flow::original(LeaseError::Conflict.into()));
                }
            } else {
                let checkpoint = checkpoints
                    .get(&file.path)
                    .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
                if file.phase == ApplicationPhase::Restored {
                    undo_checkpoint::authorize_restored(
                        checkpoint,
                        original,
                        proposal.binding.request_id,
                        &saved,
                        file,
                        restoration
                            .as_ref()
                            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?,
                    )
                    .map_err(Flow::original)?;
                } else {
                    undo_checkpoint::authorize_rebuilt(
                        checkpoint,
                        original,
                        proposal.binding.request_id,
                        &saved,
                        file,
                    )
                    .map_err(Flow::original)?;
                }
            }
            if !apply_editor::matches_text(version, &buffer.text)
                || disk.executable != version.executable
                || buffer.expected_disk_sha256 != disk.sha256
                || (recorded_source.revision != format!("sha256:{}", disk.sha256) && !saved_inverse)
                || recorded_source.resource_id != format!("{}/{}", original.manuscript, file.path)
            {
                return Err(Flow::original(LeaseError::Conflict.into()));
            }
            let index = match source_reader.index(&file.path) {
                Ok(value) => value,
                Err(Flow::Need(input)) => {
                    if tx.rollback().is_err() || !db.is_autocommit() {
                        db.require_recovery();
                        return Err(Flow::original(Error::RecoveryRequired));
                    }
                    return Err(Flow::Need(input));
                }
                Err(cause) => return Err(cause),
            };
            index
                .check_registered(&tx, original.manuscript, &file.path, disk)
                .map_err(|cause| {
                    Flow::original(cause.map(|cause| WritingSubmissionError::from(cause).into()))
                })?;
            continue;
        }
        let index = match source_reader.index(&file.path) {
            Ok(value) => value,
            Err(Flow::Need(input)) => {
                if tx.rollback().is_err() || !db.is_autocommit() {
                    db.require_recovery();
                    return Err(Flow::original(Error::RecoveryRequired));
                }
                return Err(Flow::Need(input));
            }
            Err(cause) => return Err(cause),
        };
        disk_restoration::check_plan(
            &index,
            &tx,
            original.manuscript,
            operation,
            &target,
            &actual,
            change,
            file.phase,
        )
        .map_err(Flow::original)?;
    }
    undos::require_recovery_in(&tx, original, operation)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    if saved.application.files.iter().all(|f| {
        matches!(
            f.phase,
            ApplicationPhase::Pending | ApplicationPhase::Restored
        )
    }) {
        undos::finish_rollback_in(&tx, original, operation)
            .map_err(|cause| Flow::original(Error::from(cause)))?;
    }
    let saved = undos::read_in(&tx, original, operation)
        .map_err(|cause| Flow::original(Error::from(cause)))?
        .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;
    Ok(saved)
}

#[derive(Clone)]
pub(crate) struct UndoRollbackCheck {
    pub original: ApplicationScope,
    pub operation: Uuid,
    pub allow_editor_effects: bool,
    pub checkpoints: Vec<UndoCheckpointVersion>,
}
