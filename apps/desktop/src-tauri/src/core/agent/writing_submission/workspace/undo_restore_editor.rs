//! Restore an interrupted undo's editor effect without changing original apply.
use super::restore_editor::PreparedEditorRestoration;
use super::*;
use crate::agent::{
    repository::{
        proposal_applications::{
            ApplicationPhase, ApplicationScope, ApplicationTarget, EditorRestoration,
            EditorRestorationAdmission,
        },
        proposal_undos::{self as undos, StoredUndo},
    },
    workspace::ProposalObjects,
};
use crate::ai::structured_resource_validation::Flow;
use research_agent_core::workspace::FileChange;

impl WritingSubmissionService {
    pub async fn read_undo_editor_restoration(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        path: String,
    ) -> Result<(StoredUndo, Option<EditorRestoration>), Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            let restoration = undos::read_editor_restoration_in(&tx, original, operation, &path)?;
            Ok((saved, restoration))
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Caller holds the complete manuscript lease, including paused saving.
    /// A persisted identity can only recover its exact receipt, never dispatch again.
    pub async fn prepare_undo_editor_restoration(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        restore_id: Uuid,
        current: EditorObservation,
    ) -> Result<EditorRestoreAdmission, Error> {
        self.prepare_undo_editor_restoration_with_checkpoint(
            original, operation, restore_id, current, None,
        )
        .await
    }

    pub(crate) async fn prepare_undo_editor_restoration_with_checkpoint(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        restore_id: Uuid,
        current: EditorObservation,
        proof: Option<UndoCheckpointVersion>,
    ) -> Result<EditorRestoreAdmission, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .prepare_undo_editor_compensation(
                    self.store.clone(),
                    original,
                    operation,
                    restore_id,
                    current,
                    proof,
                    self.worker_retention.clone(),
                )
                .await;
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            prepare_with_source(
                &store,
                original,
                operation,
                restore_id,
                &current,
                &proof,
                &mut store.writing_workspace_source(),
            )
            .map(|prepared| prepared.admit(restore_id, current))
            .map_err(Flow::into_static)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    /// Confirm the restoration's historical port receipt, not current buffer text.
    pub async fn confirm_undo_editor_restoration(
        &self,
        original: ApplicationScope,
        operation: Uuid,
        restore_id: Uuid,
        actual: EditorObservation,
    ) -> Result<StoredUndo, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let mut tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            let change = saved
                .changes
                .iter()
                .find(|c| c.path == actual.path)
                .ok_or(LeaseError::Conflict)?;
            if !apply_editor::matches_text(
                change.before.as_ref().ok_or(LeaseError::Conflict)?,
                &actual.text,
            ) {
                return Err(LeaseError::Conflict.into());
            }
            undos::acknowledge_editor_restored_in(
                &mut tx,
                original,
                operation,
                &actual.path,
                restore_id,
                &actual.version,
            )?;
            let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            if saved.application.files.iter().all(|f| {
                matches!(
                    f.phase,
                    ApplicationPhase::Pending | ApplicationPhase::Restored
                )
            }) {
                undos::finish_rollback_in(&tx, original, operation)?;
            }
            let saved = undos::read_in(&tx, original, operation)?.ok_or(LeaseError::Conflict)?;
            tx.commit().map_err(|_| Error::RecoveryRequired)?;
            Ok(saved)
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(crate) fn prepare_with_source<S: super::restoration_source::RestorationSource>(
    store: &AppStore,
    original: ApplicationScope,
    operation: Uuid,
    restore_id: Uuid,
    current: &EditorObservation,
    proof: &Option<UndoCheckpointVersion>,
    source_reader: &mut S,
) -> Result<PreparedEditorRestoration, Flow<Error, S::Pause>> {
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
    if let Some(restoration) =
        undos::read_editor_restoration_in(&tx, original, operation, &current.path)
            .map_err(|cause| Flow::original(Error::from(cause)))?
    {
        if restoration.id != restore_id {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
        return Ok(PreparedEditorRestoration::Existing {
            application: saved.application,
            restoration,
        });
    }
    let result = (|| {
        let file = saved
            .application
            .files
            .iter()
            .find(|f| f.path == current.path)
            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        let ApplicationTarget::Editor {
            expected,
            result_revision: Some(revision),
        } = &file.target
        else {
            return Err(Flow::original(LeaseError::Conflict.into()));
        };
        if file.phase != ApplicationPhase::Acknowledged
            || (expected.session_id == current.version.session_id
                && *revision != current.version.revision)
        {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
        if expected.session_id != current.version.session_id || proof.is_some() {
            let checkpoint = undo_checkpoint::read_editor_with(
                current,
                proof
                    .as_ref()
                    .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?,
                || source_reader.checkpoint(&tx, original.manuscript, &current.path),
            )?;
            let proposal = crate::agent::repository::change_sets::read_file_operation_in(
                &tx,
                original.conversation,
                original.manuscript,
                original.proposal,
            )
            .map_err(|cause| Flow::original(Error::from(cause)))?;
            undo_checkpoint::authorize_rebuilt(
                &checkpoint,
                original,
                proposal.binding.request_id,
                &saved,
                file,
            )
            .map_err(Flow::original)?;
            undo_checkpoint::check_editor_disk(
                store,
                &tx,
                original,
                &saved,
                file,
                &checkpoint,
                source_reader,
            )?;
        }
        let change = saved
            .changes
            .iter()
            .find(|c| c.path == current.path)
            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        if !apply_editor::matches_text(
            change
                .after
                .as_ref()
                .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?,
            &current.text,
        ) {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
        let objects = ProposalObjects::open(
            &store
                .content_directory()
                .canonicalize()
                .map_err(|_| Flow::original(Error::Storage))?,
        )
        .map_err(|cause| Flow::original(Error::from(cause)))?;
        let reverse = FileChange {
            before: change.after.clone(),
            after: change.before.clone(),
            ..change.clone()
        };
        let text =
            apply_editor::replacement_text(&tx, &objects, &reverse).map_err(Flow::original)?;
        if !matches!(
            undos::begin_editor_restoration_in(
                &tx,
                original,
                operation,
                &current.path,
                restore_id,
                &current.version
            )
            .map_err(|cause| Flow::original(Error::from(cause)))?,
            EditorRestorationAdmission::Created(_)
        ) {
            return Err(Flow::original(Error::RecoveryRequired));
        }
        Ok(text)
    })();
    match result {
        Ok(text) => {
            tx.commit()
                .map_err(|_| Flow::original(Error::RecoveryRequired))?;
            Ok(PreparedEditorRestoration::Dispatch(text))
        }
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Flow::original(Error::RecoveryRequired));
            }
            Err(Flow::Need(input))
        }
        Err(cause) => Err(cause),
    }
}
