//! Compensating editor transactions for interrupted applications; never generic undo.
use super::*;
use crate::agent::{
    repository::{
        change_sets,
        proposal_applications::{
            self as applications, ApplicationPhase, ApplicationScope, ApplicationTarget,
            EditorRestoration, StoredApplication,
        },
    },
    workspace::ProposalObjects,
};
use crate::ai::structured_resource_validation::Flow;
use apply_editor::matches_text;
use research_agent_core::workspace::FileChange;

pub enum EditorRestoreAdmission {
    Dispatch(EditorDispatch),
    /// Read the operation-specific port receipt; never replay the edit.
    Existing {
        application: StoredApplication,
        restoration: EditorRestoration,
    },
}

/// Keep the caller's observation owned once across source pauses. Only the
/// committed transaction's result can be paired with it for dispatch.
pub(crate) enum PreparedEditorRestoration {
    Dispatch(String),
    Existing {
        application: StoredApplication,
        restoration: EditorRestoration,
    },
}
impl PreparedEditorRestoration {
    pub(crate) fn admit(
        self,
        restore_id: Uuid,
        current: EditorObservation,
    ) -> EditorRestoreAdmission {
        match self {
            Self::Dispatch(text) => EditorRestoreAdmission::Dispatch(EditorDispatch {
                operation: restore_id,
                expected: current,
                text,
            }),
            Self::Existing {
                application,
                restoration,
            } => EditorRestoreAdmission::Existing {
                application,
                restoration,
            },
        }
    }
}

impl WritingSubmissionService {
    pub async fn read_editor_restoration(
        &self,
        scope: ApplicationScope,
        path: String,
    ) -> Result<(StoredApplication, Option<EditorRestoration>), Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let tx = db.transaction().map_err(|_| Error::Storage)?;
            let restoration = applications::read_editor_restoration_in(&tx, scope, &path)?;
            let saved = applications::read_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )?;
            Ok((saved, restoration))
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    /// Caller holds the complete manuscript lease and pauses saving. Check the
    /// exact post-apply buffer before committing intent; no disk fallback.
    pub async fn prepare_editor_restoration(
        &self,
        scope: ApplicationScope,
        restore_id: Uuid,
        current: EditorObservation,
    ) -> Result<EditorRestoreAdmission, Error> {
        self.prepare_editor_restoration_with_checkpoint(scope, restore_id, current, None)
            .await
    }
    pub(crate) async fn prepare_editor_restoration_with_checkpoint(
        &self,
        scope: ApplicationScope,
        restore_id: Uuid,
        current: EditorObservation,
        proof: Option<UndoCheckpointVersion>,
    ) -> Result<EditorRestoreAdmission, Error> {
        if self.store.writing_workspace_source().is_none()
            && let Some(binding) = &self.runtime_context
        {
            return binding
                .prepare_editor_compensation(
                    self.store.clone(),
                    scope,
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
                scope,
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

    /// `actual` is the receipt for restore_id, not a guess based on current text.
    /// A lost confirmation can be retried with that same receipt after later edits.
    pub async fn confirm_editor_restoration(
        &self,
        scope: ApplicationScope,
        restore_id: Uuid,
        actual: EditorObservation,
    ) -> Result<StoredApplication, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|_| Error::Storage)?;
            let mut tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::Storage)?;
            let proposal = change_sets::read_file_operation_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
            )?;
            let change = proposal
                .changes
                .iter()
                .find(|change| change.path == actual.path)
                .ok_or(LeaseError::Conflict)?;
            if !matches_text(
                change.before.as_ref().ok_or(LeaseError::Conflict)?,
                &actual.text,
            ) {
                return Err(LeaseError::Conflict.into());
            }
            applications::acknowledge_editor_restored_in(
                &mut tx,
                scope,
                &actual.path,
                restore_id,
                &actual.version,
            )?;
            let saved = applications::read_in(
                &tx,
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )?;
            if saved.files.iter().all(|file| {
                matches!(
                    file.phase,
                    ApplicationPhase::Pending | ApplicationPhase::Restored
                )
            }) {
                applications::finish_rollback_in(&mut tx, scope)?;
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
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub(crate) fn prepare_with_source<S: super::restoration_source::RestorationSource>(
    store: &AppStore,
    scope: ApplicationScope,
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
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Flow::original(Error::Storage))?;
    if let Some(restoration) = applications::read_editor_restoration_in(&tx, scope, &current.path)
        .map_err(|cause| Flow::original(Error::from(cause)))?
    {
        if restoration.id != restore_id {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
        let application = applications::read_in(
            &tx,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .map_err(|cause| Flow::original(Error::from(cause)))?;
        return Ok(PreparedEditorRestoration::Existing {
            application,
            restoration,
        });
    }
    let result = (|| {
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
            .find(|file| file.path == current.path)
            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        let ApplicationTarget::Editor {
            expected,
            result_revision: Some(revision),
        } = &file.target
        else {
            return Err(Flow::original(LeaseError::Conflict.into()));
        };
        if file.phase != ApplicationPhase::Acknowledged
            || (current.version.session_id == expected.session_id
                && current.version.revision != *revision)
        {
            return Err(Flow::original(LeaseError::Conflict.into()));
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
            .find(|change| change.path == current.path)
            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        if current.version.session_id != expected.session_id || proof.is_some() {
            let checkpoint = undo_checkpoint::read_editor_with(
                current,
                proof
                    .as_ref()
                    .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?,
                || source_reader.checkpoint(&tx, scope.manuscript, &current.path),
            )?;
            application_checkpoint::authorize_rebuilt(
                &checkpoint,
                scope,
                proposal.binding.request_id,
                change,
                file,
            )
            .map_err(Flow::original)?;
            application_checkpoint::check_editor_disk(
                store,
                &tx,
                scope,
                change,
                &checkpoint,
                source_reader,
            )?;
        }
        let after = change
            .after
            .as_ref()
            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        if !matches_text(after, &current.text) {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
        let objects = ProposalObjects::open(
            &store
                .content_directory()
                .canonicalize()
                .map_err(|_| Flow::original(Error::Storage))?,
        )
        .map_err(|cause| Flow::original(Error::from(cause)))?;
        // Reuse replacement constraints and object verification in the reverse direction.
        let reverse = FileChange {
            before: change.after.clone(),
            after: change.before.clone(),
            ..change.clone()
        };
        let text =
            apply_editor::replacement_text(&tx, &objects, &reverse).map_err(Flow::original)?;
        applications::require_recovery_in(&mut tx, scope)
            .map_err(|cause| Flow::original(Error::from(cause)))?;
        if !matches!(
            applications::begin_editor_restoration_in(
                &tx,
                scope,
                &current.path,
                restore_id,
                &current.version
            )
            .map_err(|cause| Flow::original(Error::from(cause)))?,
            applications::EditorRestorationAdmission::Created(_)
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
