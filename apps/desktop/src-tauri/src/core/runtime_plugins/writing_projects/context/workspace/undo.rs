//! Adapt the original undo lifecycle to the same request-local source worker.
use super::*;
use crate::agent::{
    repository::{proposal_applications::ApplicationScope, proposal_undos::StoredUndo},
    writing_submission::workspace::undo_admission::{self, Admission},
};

impl Binding {
    pub(crate) async fn admit_undo(
        &self,
        store: AppStore,
        request: WritingWorkspaceRequest,
        admission: Admission,
        retention: WorkerRetention,
    ) -> Result<StoredUndo, Error> {
        self.with_snapshot_source(
            store,
            admission.operation,
            request,
            retention,
            move |owner, _, source| {
                undo_admission::admit(
                    owner,
                    admission.original,
                    admission.operation,
                    admission.kind,
                    |db, root, identity| {
                        source.capture_undo(db, root, identity, &admission.checkpoints)
                    },
                )
            },
        )
        .await
    }
    pub(crate) async fn apply_undo_file(
        &self,
        store: AppStore,
        original: ApplicationScope,
        operation: Uuid,
        path: String,
        retention: WorkerRetention,
    ) -> Result<StoredUndo, Error> {
        self.with_source(
            store,
            WorkspaceSource::empty(self.clone(), operation),
            retention,
            move |owner, source| {
                crate::agent::writing_submission::workspace::undo_disk::apply_with_index(
                    owner,
                    original,
                    operation,
                    &path,
                    |path| source.capture_index(path),
                )
            },
        )
        .await
    }
    pub(crate) async fn reconcile_undo_file(
        &self,
        store: AppStore,
        original: ApplicationScope,
        operation: Uuid,
        path: String,
        retention: WorkerRetention,
    ) -> Result<StoredUndo, Error> {
        self.with_source(
            store,
            WorkspaceSource::reconciliation(self.clone(), operation),
            retention,
            move |owner, source| {
                crate::agent::writing_submission::workspace::undo_reconcile::reconcile_with_index(
                    owner,
                    original,
                    operation,
                    &path,
                    |path| source.capture_index(path),
                )
            },
        )
        .await
    }
}
