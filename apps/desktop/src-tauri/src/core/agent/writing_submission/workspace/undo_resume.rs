//! Explicit re-admission after all dispatched undo effects have been reconciled.
use super::*;
use crate::agent::repository::{
    change_sets,
    proposal_applications::{ApplicationPhase, ApplicationScope, ApplicationTarget},
    proposal_undos::{self, StoredUndo},
};
use crate::agent::workspace::ProposalObjects;
use crate::agent::{
    writing_checkpoint_contracts::DocumentCheckpoint, writing_workspace::WritingWorkspaceSnapshot,
};
use crate::ai::structured_resource_validation::Flow;
use std::{collections::BTreeMap, path::Path};

pub(crate) type UndoPreflightSnapshot = (
    WritingWorkspaceSnapshot,
    BTreeMap<String, DocumentCheckpoint>,
);

impl WritingSubmissionService {
    /// The caller owns complete live-buffer coverage and the manuscript lease
    /// through subsequent dispatch. Never repeats an acknowledged file effect.
    pub async fn resume_proposal_undo(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
    ) -> Result<StoredUndo, Error> {
        self.resume_proposal_undo_with_checkpoints(original, operation, request, vec![])
            .await
    }

    pub(crate) async fn resume_proposal_undo_with_checkpoints(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        request: WritingWorkspaceRequest,
        checkpoints: Vec<UndoCheckpointVersion>,
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
                        kind: undo_admission::Kind::Resume,
                        checkpoints,
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
                undo_admission::Kind::Resume,
                |db, root, identity| {
                    let records = undo_checkpoint::read_in(
                        db,
                        store.writing_checkpoint_source(),
                        &request,
                        &checkpoints,
                    )
                    .map_err(Flow::original)?;
                    let snapshot = preflight::static_snapshot(&store, db, root, identity, request)?;
                    Ok((snapshot, records))
                },
            )
            .map_err(Flow::<Error, std::convert::Infallible>::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

/// Complete source preflight is shared by explicit start and recovery; only a
/// reconciled plan can admit Pending owners with reliable current checkpoint CAS.
pub(crate) fn preflight_with_capture<P>(
    store: &AppStore,
    tx: &mut rusqlite::Transaction<'_>,
    original: ApplicationScope,
    saved: &StoredUndo,
    capture: impl FnOnce(
        &rusqlite::Connection,
        &Path,
        WorkspaceIdentity,
    ) -> Result<UndoPreflightSnapshot, Flow<Error, P>>,
) -> Result<(), Flow<Error, P>> {
    let (proposal, root) = (|| -> Result<_, Error> {
        let proposal = change_sets::read_file_operation_in(
            tx,
            original.conversation,
            original.manuscript,
            original.proposal,
        )?;
        let root = store
            .manuscript_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?;

        Ok((proposal, root))
    })()
    .map_err(Flow::original)?;
    let (snapshot, checkpoints) = capture(
        tx,
        &root,
        WorkspaceIdentity {
            conversation_id: original.conversation,
            turn_id: proposal.binding.turn_id,
            lease_id: proposal.binding.lease_id,
            environment_id: format!("manuscript:{}", original.manuscript),
        },
    )?;
    (|| -> Result<(), Error> {
        let objects = ProposalObjects::open(
            &store
                .content_directory()
                .canonicalize()
                .map_err(|_| Error::Storage)?,
        )?;
        let all_pending = !saved.application.files.is_empty()
            && saved
                .application
                .files
                .iter()
                .all(|file| file.phase == ApplicationPhase::Pending);
        let mut targets = std::collections::BTreeMap::new();
        for change in &saved.changes {
            for version in [&change.before, &change.after].into_iter().flatten() {
                let path: String = tx
                    .query_row(
                        "select relative_path from content_objects where sha256=?1",
                        [&version.sha256],
                        |row| row.get(0),
                    )
                    .map_err(|_| Error::Storage)?;
                objects.verify_registered(version, &path)?;
            }
            let file = saved
                .application
                .files
                .iter()
                .find(|f| f.path == change.path)
                .ok_or(LeaseError::Conflict)?;
            let version = match file.phase {
                ApplicationPhase::Pending => &change.before,
                ApplicationPhase::Acknowledged => &change.after,
                _ => return Err(LeaseError::Conflict.into()),
            };
            let actual = snapshot
                .manifest
                .files
                .iter()
                .find(|f| f.file.path == change.path);
            if version.as_ref() != actual.map(|f| &f.file.version) {
                return Err(LeaseError::Conflict.into());
            }
            if actual.is_none() {
                store
                    .check_absent_writing_file(&root, original.manuscript, &change.path)
                    .map_err(WritingSubmissionError::from)?;
            }
            match &file.target {
                ApplicationTarget::Disk => {
                    if actual.is_some_and(|f| f.source.editor.is_some()) {
                        return Err(LeaseError::Conflict.into());
                    }
                }
                ApplicationTarget::Editor {
                    expected,
                    result_revision,
                } => {
                    let current = actual
                        .and_then(|f| f.source.editor.as_ref())
                        .ok_or(LeaseError::Conflict)?;
                    let revision = if file.phase == ApplicationPhase::Pending {
                        expected.revision
                    } else {
                        result_revision.ok_or(LeaseError::Conflict)?
                    };
                    let same_owner = current.session_id == expected.session_id;
                    if same_owner && current.revision != revision {
                        return Err(LeaseError::Conflict.into());
                    }
                    if !same_owner && !all_pending {
                        undo_checkpoint::authorize_rebuilt(
                            checkpoints.get(&file.path).ok_or(LeaseError::Conflict)?,
                            original,
                            proposal.binding.request_id,
                            saved,
                            file,
                        )?;
                    }
                }
            }
            if file.phase == ApplicationPhase::Pending {
                let expected_source = change.source.as_ref();
                let current_source = actual.map(|file| &file.source);
                if expected_source.map(|s| (&s.resource_id, &s.revision, s.editor.is_some()))
                    != current_source.map(|s| (&s.resource_id, &s.revision, s.editor.is_some()))
                {
                    return Err(LeaseError::Conflict.into());
                }
            }
            targets.insert(
                file.path.clone(),
                actual.and_then(|file| file.source.editor.clone()),
            );
        }
        if saved
            .application
            .files
            .iter()
            .any(|file| file.phase == ApplicationPhase::Pending)
        {
            proposal_undos::rebind_pending_editor_targets_in(
                tx,
                original,
                saved.application.operation_id,
                &targets,
            )?;
        }
        Ok(())
    })()
    .map_err(Flow::original)
}
