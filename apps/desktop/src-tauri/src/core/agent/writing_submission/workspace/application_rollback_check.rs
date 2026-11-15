//! Original full-plan compensation admission; no historical target or receipt rebinding.
use super::*;
use crate::agent::{
    repository::{
        change_sets,
        proposal_applications::{
            self as applications, ApplicationPhase, ApplicationScope, ApplicationStatus,
            ApplicationTarget, StoredApplication,
        },
    },
    workspace::{DiskTarget, ProposalObjects},
};
use crate::{
    agent::writing_submission::workspace::restoration_source::RestorationSource,
    ai::structured_resource_validation::Flow,
};

#[derive(Clone, Copy)]
pub(crate) enum RollbackStage {
    BeforeEditorCompensation,
    BeforeDiskCompensation,
}

impl WritingSubmissionService {
    pub(crate) async fn prepare_proposal_rollback_with_checkpoints(
        &self,
        scope: ApplicationScope,
        request: WritingWorkspaceRequest,
        proofs: Vec<UndoCheckpointVersion>,
    ) -> Result<StoredApplication, Error> {
        self.check_application_rollback(
            scope,
            request,
            proofs,
            RollbackStage::BeforeEditorCompensation,
        )
        .await
    }
    pub(crate) async fn rollback_proposal_remainder_with_checkpoints(
        &self,
        scope: ApplicationScope,
        request: WritingWorkspaceRequest,
        proofs: Vec<UndoCheckpointVersion>,
    ) -> Result<StoredApplication, Error> {
        let state = self
            .check_application_rollback(
                scope,
                request,
                proofs,
                RollbackStage::BeforeDiskCompensation,
            )
            .await?;
        if state.status == ApplicationStatus::RolledBack {
            return Ok(state);
        }
        self.rollback_proposal_remainder(scope).await
    }
    async fn check_application_rollback(
        &self,
        scope: ApplicationScope,
        request: WritingWorkspaceRequest,
        proofs: Vec<UndoCheckpointVersion>,
        stage: RollbackStage,
    ) -> Result<StoredApplication, Error> {
        if request.manuscript_id != scope.manuscript || request.buffers.len() > 20_000 {
            return Err(LeaseError::Conflict.into());
        }
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .check_application_compensation(
                    self.store.clone(),
                    scope,
                    request,
                    proofs,
                    stage,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            check_with_source(
                &store,
                scope,
                &request,
                &proofs,
                stage,
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
    scope: ApplicationScope,
    request: &WritingWorkspaceRequest,
    proofs: &[UndoCheckpointVersion],
    stage: RollbackStage,
    source_reader: &mut S,
) -> Result<StoredApplication, Flow<Error, S::Pause>> {
    let _content = store
        .content_read()
        .map_err(|_| Flow::original(Error::Storage))?;
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
    if saved.status == ApplicationStatus::RolledBack {
        return Ok(saved);
    }
    if !matches!(
        saved.status,
        ApplicationStatus::Prepared
            | ApplicationStatus::Applying
            | ApplicationStatus::RecoveryRequired
    ) || saved.files.iter().any(|f| {
        f.phase == ApplicationPhase::Conflict
            || (f.target == ApplicationTarget::Disk
                && request.buffers.iter().any(|b| b.path == f.path))
    }) {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    let checkpoints = match undo_checkpoint::read_with(request, proofs, |manuscript, path| {
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
    let proposal = change_sets::read_file_operation_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    let source = store
        .manuscript_directory()
        .canonicalize()
        .map_err(|_| Flow::original(Error::Storage))?
        .join(scope.manuscript.to_string())
        .join("source");
    let objects = ProposalObjects::open(
        &store
            .content_directory()
            .canonicalize()
            .map_err(|_| Flow::original(Error::Storage))?,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    if saved.files.len() != proposal.changes.len() {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    for (file, change) in saved.files.iter().zip(&proposal.changes) {
        if file.path != change.path {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
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
            if request
                .buffers
                .iter()
                .filter(|b| b.path == file.path)
                .count()
                != 1
            {
                return Err(Flow::original(LeaseError::Conflict.into()));
            }
            let buffer = request
                .buffers
                .iter()
                .find(|b| b.path == file.path)
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            let restoration = applications::read_editor_restoration_in(&tx, scope, &file.path)
                .map_err(|cause| Flow::original(Error::from(cause)))?;
            let owner = restoration
                .as_ref()
                .map(|r| r.expected.session_id)
                .unwrap_or(expected.session_id);
            let (version, revision) = match file.phase {
                ApplicationPhase::Pending if restoration.is_none() => {
                    (&change.before, expected.revision)
                }
                ApplicationPhase::Acknowledged
                    if matches!(stage, RollbackStage::BeforeEditorCompensation)
                        && restoration.is_none() =>
                {
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
            if buffer.editor.session_id == owner {
                if buffer.editor.revision != revision {
                    return Err(Flow::original(LeaseError::Conflict.into()));
                }
            } else {
                let checkpoint = checkpoints
                    .get(&file.path)
                    .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
                match file.phase {
                    ApplicationPhase::Acknowledged => application_checkpoint::authorize_rebuilt(
                        checkpoint,
                        scope,
                        proposal.binding.request_id,
                        change,
                        file,
                    )
                    .map_err(Flow::original)?,
                    ApplicationPhase::Restored => application_checkpoint::authorize_restored(
                        checkpoint,
                        scope,
                        proposal.binding.request_id,
                        change,
                        restoration
                            .as_ref()
                            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?,
                    )
                    .map_err(Flow::original)?,
                    // No apply was dispatched for a Pending target. Its exact
                    // frozen before and current checkpoint CAS suffice; no receipt is invented.
                    ApplicationPhase::Pending => {}
                    _ => return Err(Flow::original(LeaseError::Conflict.into())),
                }
            }
            let disk = actual
                .as_ref()
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            let recorded = change
                .source
                .as_ref()
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
            let saved_result = matches!(
                file.phase,
                ApplicationPhase::Acknowledged | ApplicationPhase::Restored
            ) && change.after.as_ref() == Some(disk);
            if !apply_editor::matches_text(version, &buffer.text)
                || disk.executable != version.executable
                || buffer.expected_disk_sha256 != disk.sha256
                || recorded.resource_id != format!("{}/{}", scope.manuscript, file.path)
                || (recorded.revision != format!("sha256:{}", disk.sha256) && !saved_result)
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
                .check_registered(&tx, scope.manuscript, &file.path, disk)
                .map_err(|cause| {
                    Flow::original(cause.map(|cause| WritingSubmissionError::from(cause).into()))
                })?;
        } else {
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
                scope.manuscript,
                scope.operation,
                &target,
                &actual,
                change,
                file.phase,
            )
            .map_err(Flow::original)?;
        }
    }
    applications::require_recovery_in(&mut tx, scope)
        .map_err(|cause| Flow::original(Error::from(cause)))?;
    if saved.files.iter().all(|f| {
        matches!(
            f.phase,
            ApplicationPhase::Pending | ApplicationPhase::Restored
        )
    }) {
        applications::finish_rollback_in(&mut tx, scope)
            .map_err(|cause| Flow::original(Error::from(cause)))?;
    }
    let saved = applications::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .map_err(|cause| Flow::original(Error::from(cause)))?;
    tx.commit()
        .map_err(|_| Flow::original(Error::RecoveryRequired))?;
    Ok(saved)
}
