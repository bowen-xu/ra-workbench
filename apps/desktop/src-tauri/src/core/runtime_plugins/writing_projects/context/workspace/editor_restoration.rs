//! Restore rebuilt editors through the original Core transactions and receipts.
use super::*;
use crate::agent::{
    repository::proposal_applications::ApplicationScope,
    writing_submission::workspace::{
        EditorObservation, EditorRestoreAdmission, UndoCheckpointVersion, restore_editor,
        undo_restore_editor,
    },
};

impl Binding {
    pub(crate) async fn prepare_editor_compensation(
        &self,
        store: AppStore,
        scope: ApplicationScope,
        restore_id: Uuid,
        current: EditorObservation,
        proof: Option<UndoCheckpointVersion>,
        retention: WorkerRetention,
    ) -> Result<EditorRestoreAdmission, Error> {
        self.with_source(
            store,
            WorkspaceSource::editor(self.clone(), scope.operation, current),
            retention,
            move |owner, source| {
                source.prepare_editor(restore_id, |current, source| {
                    restore_editor::prepare_with_source(
                        owner, scope, restore_id, current, &proof, source,
                    )
                })
            },
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn prepare_undo_editor_compensation(
        &self,
        store: AppStore,
        original: ApplicationScope,
        operation: Uuid,
        restore_id: Uuid,
        current: EditorObservation,
        proof: Option<UndoCheckpointVersion>,
        retention: WorkerRetention,
    ) -> Result<EditorRestoreAdmission, Error> {
        self.with_source(
            store,
            WorkspaceSource::editor(self.clone(), operation, current),
            retention,
            move |owner, source| {
                source.prepare_editor(restore_id, |current, source| {
                    undo_restore_editor::prepare_with_source(
                        owner, original, operation, restore_id, current, &proof, source,
                    )
                })
            },
        )
        .await
    }
}
