//! Original discovery and delivery transactions retain their command errors.
use super::*;
use crate::agent::writing_application_commands::{self as commands, discovery, undo};
use crate::agent::writing_workspace_commands::Error as CommandError;

fn command_error(source: Option<&WorkspaceSource>, cause: Error) -> CommandError {
    commands::checkpoint_source_error(cause, source.and_then(|source| source.command_scope))
}

impl Binding {
    pub(crate) async fn verify_running_proof(
        &self,
        store: AppStore,
        request: crate::agent::writing_delivery_commands::Acknowledge,
        review: crate::agent::writing_submission::workspace::WorkspaceReview,
        delivery_id: Uuid,
        provider_call_id: Option<String>,
        retention: WorkerRetention,
    ) -> Result<crate::agent::writing_delivery_commands::RunningProof, CommandError> {
        let review = std::sync::Arc::new(review);
        self.with_mapped_source(
            store,
            WorkspaceSource::empty(self.clone(), delivery_id),
            retention,
            move |owner, source| {
                crate::agent::writing_delivery_commands::proof_with_checkpoints(
                    owner,
                    &request,
                    &review,
                    delivery_id,
                    provider_call_id.as_deref(),
                    |db, scope, path| source.command_checkpoint(db, scope, path),
                )
            },
            command_error,
        )
        .await
    }

    pub(crate) async fn list_pending_applications(
        &self,
        store: AppStore,
        request: discovery::ListRequest,
    ) -> Result<discovery::Page, CommandError> {
        self.with_mapped_source(
            store,
            WorkspaceSource::empty(self.clone(), Uuid::nil()),
            WorkerRetention::default(),
            move |owner, source| {
                discovery::list_with_checkpoints(owner, &request, |db, scope, path| {
                    source.command_checkpoint(db, scope, path)
                })
            },
            command_error,
        )
        .await
    }

    pub(crate) async fn list_pending_undos(
        &self,
        store: AppStore,
        request: undo::discovery::ListRequest,
    ) -> Result<undo::discovery::Page, CommandError> {
        self.with_mapped_source(
            store,
            WorkspaceSource::empty(self.clone(), Uuid::nil()),
            WorkerRetention::default(),
            move |owner, source| {
                undo::discovery::list_with_checkpoints(owner, &request, |db, scope, path| {
                    source.command_checkpoint(db, scope, path)
                })
            },
            command_error,
        )
        .await
    }
}
