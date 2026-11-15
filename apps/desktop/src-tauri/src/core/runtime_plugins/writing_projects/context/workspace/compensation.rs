//! Reuse the original compensation transactions with request-local source reads.
use super::*;
use crate::agent::{
    repository::{
        proposal_applications::{ApplicationScope, StoredApplication},
        proposal_undos::StoredUndo,
    },
    writing_submission::workspace::{
        UndoCheckpointVersion,
        application_rollback_check::{self, RollbackStage},
        undo_rollback_plan::{self, UndoRollbackCheck},
    },
};

impl Binding {
    pub(crate) async fn check_application_compensation(
        &self,
        store: AppStore,
        scope: ApplicationScope,
        request: WritingWorkspaceRequest,
        proofs: Vec<UndoCheckpointVersion>,
        stage: RollbackStage,
        retention: WorkerRetention,
    ) -> Result<StoredApplication, Error> {
        self.with_snapshot_source(
            store,
            scope.operation,
            request,
            retention,
            move |owner, _, source| {
                source.with_request(|request, source| {
                    application_rollback_check::check_with_source(
                        owner, scope, request, &proofs, stage, source,
                    )
                })
            },
        )
        .await
    }
    pub(crate) async fn check_undo_compensation(
        &self,
        store: AppStore,
        request: WritingWorkspaceRequest,
        input: UndoRollbackCheck,
        retention: WorkerRetention,
    ) -> Result<StoredUndo, Error> {
        self.with_snapshot_source(
            store,
            input.operation,
            request,
            retention,
            move |owner, _, source| {
                source.with_request(|request, source| {
                    undo_rollback_plan::check_with_source(owner, request, &input, source)
                })
            },
        )
        .await
    }
    pub(crate) async fn rollback_application_file(
        &self,
        store: AppStore,
        scope: ApplicationScope,
        path: String,
        kind: crate::agent::writing_submission::workspace::rollback_removed::RollbackKind,
        retention: WorkerRetention,
    ) -> Result<StoredApplication, Error> {
        self.with_source(
            store,
            WorkspaceSource::reconciliation(self.clone(), scope.operation),
            retention,
            move |owner, source| {
                crate::agent::writing_submission::workspace::rollback_removed::rollback_with_index(
                    owner,
                    scope,
                    &path,
                    kind,
                    |path| source.capture_index(path),
                )
            },
        )
        .await
    }
    pub(crate) async fn rollback_undo_file(
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
                crate::agent::writing_submission::workspace::undo_rollback::rollback_with_index(
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
