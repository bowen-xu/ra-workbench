//! Owned adaptation of the original Core snapshot and file-application phases.
use super::Binding;
use crate::{
    agent::{
        repository::WorkerRetention,
        writing_context::ContextNeed,
        writing_submission::{
            WritingSubmissionReceipt,
            workspace::{
                Error, PreparedWorkspace,
                preparation::{self, Attempt},
                successor_preparation::{self, Successor},
            },
        },
        writing_workspace::WritingWorkspaceRequest,
    },
    ai::structured_resource_validation::Flow,
    storage::AppStore,
};
use uuid::Uuid;
#[path = "workspace/source.rs"]
mod source;
use source::WorkspaceSource;

#[derive(Clone, Copy)]
enum CopyPreparation {
    Initial(Uuid),
    Successor {
        context_snapshot_id: Uuid,
        successor: Successor,
    },
}

impl Binding {
    pub(crate) async fn prepare_workspace(
        &self,
        store: AppStore,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
        request: WritingWorkspaceRequest,
        retention: WorkerRetention,
    ) -> Result<PreparedWorkspace, Error> {
        self.prepare_copy(
            store,
            receipt,
            request,
            retention,
            CopyPreparation::Initial(context_snapshot_id),
        )
        .await
    }
    pub(crate) async fn prepare_next_workspace(
        &self,
        store: AppStore,
        receipt: WritingSubmissionReceipt,
        context_snapshot_id: Uuid,
        successor: Successor,
        request: WritingWorkspaceRequest,
        retention: WorkerRetention,
    ) -> Result<PreparedWorkspace, Error> {
        self.prepare_copy(
            store,
            receipt,
            request,
            retention,
            CopyPreparation::Successor {
                context_snapshot_id,
                successor,
            },
        )
        .await
    }
    pub(crate) async fn begin_application(
        &self,
        store: AppStore,
        conversation: Uuid,
        proposal: Uuid,
        operation: Uuid,
        request: WritingWorkspaceRequest,
        retention: WorkerRetention,
    ) -> Result<crate::agent::repository::proposal_applications::ApplicationAdmission, Error> {
        self.with_snapshot_source(
            store,
            operation,
            request,
            retention,
            move |owner, manuscript, source| {
                crate::agent::writing_submission::workspace::application::begin_with_snapshot(
                    owner,
                    conversation,
                    proposal,
                    operation,
                    manuscript,
                    |db, root, scope| source.capture(db, root, scope),
                )
            },
        )
        .await
    }
    pub(crate) async fn resume_application(
        &self,
        store: AppStore,
        scope: crate::agent::repository::proposal_applications::ApplicationScope,
        request: WritingWorkspaceRequest,
        retention: WorkerRetention,
    ) -> Result<crate::agent::repository::proposal_applications::StoredApplication, Error> {
        self.with_snapshot_source(
            store,
            scope.operation,
            request,
            retention,
            move |owner, manuscript, source| {
                crate::agent::writing_submission::workspace::application::resume_with_snapshot(
                    owner,
                    scope,
                    manuscript,
                    |db, root, identity| source.capture(db, root, identity),
                )
            },
        )
        .await
    }
    async fn prepare_copy(
        &self,
        store: AppStore,
        receipt: WritingSubmissionReceipt,
        request: WritingWorkspaceRequest,
        retention: WorkerRetention,
        kind: CopyPreparation,
    ) -> Result<PreparedWorkspace, Error> {
        self.with_snapshot_source(
            store,
            receipt.request_id,
            request,
            retention,
            move |owner, manuscript, source| {
                let result = match kind {
                    CopyPreparation::Initial(context) => preparation::prepare(
                        owner,
                        receipt,
                        context,
                        manuscript,
                        |db, root, scope| source.capture(db, root, scope),
                    ),
                    CopyPreparation::Successor {
                        context_snapshot_id,
                        successor,
                    } => successor_preparation::prepare(
                        owner,
                        receipt,
                        context_snapshot_id,
                        successor,
                        manuscript,
                        |db, root, scope| source.capture(db, root, scope),
                    ),
                }
                .map_err(Flow::original)?;
                match result {
                    Attempt::Ready(prepared) => Ok(*prepared),
                    Attempt::Need(input) => Err(Flow::Need(input)),
                }
            },
        )
        .await
    }

    // All four original snapshot consumers retain the same request-local
    // reader and actual-worker leases. The callback is the original Core
    // transaction; only its confirmed pre-side-effect pause may be retried.
    async fn with_snapshot_source<T, F>(
        &self,
        store: AppStore,
        operation: Uuid,
        request: WritingWorkspaceRequest,
        retention: WorkerRetention,
        attempt: F,
    ) -> Result<T, Error>
    where
        T: Send + 'static,
        F: Fn(&AppStore, Uuid, &mut WorkspaceSource) -> Result<T, Flow<Error, ContextNeed>>
            + Clone
            + Send
            + 'static,
    {
        let manuscript = request.manuscript_id;
        self.with_source(
            store,
            WorkspaceSource::new(self.clone(), operation, request),
            retention,
            move |owner, source| attempt(owner, manuscript, source),
        )
        .await
    }

    pub(crate) async fn apply_disk_file(
        &self,
        store: AppStore,
        scope: crate::agent::repository::proposal_applications::ApplicationScope,
        path: String,
        added_only: bool,
        retention: WorkerRetention,
    ) -> Result<crate::agent::repository::proposal_applications::StoredApplication, Error> {
        self.with_source(
            store,
            WorkspaceSource::empty(self.clone(), scope.operation),
            retention,
            move |owner, source| {
                crate::agent::writing_submission::workspace::apply_disk::apply_with_index(
                    owner,
                    scope,
                    &path,
                    added_only,
                    |path| source.capture_index(path),
                )
            },
        )
        .await
    }

    pub(crate) async fn reconcile_disk_file(
        &self,
        store: AppStore,
        scope: crate::agent::repository::proposal_applications::ApplicationScope,
        path: String,
        added_only: bool,
        retention: WorkerRetention,
    ) -> Result<crate::agent::repository::proposal_applications::StoredApplication, Error> {
        let source = WorkspaceSource::reconciliation(self.clone(), scope.operation);
        self.with_source(store, source, retention, move |owner, source| {
            crate::agent::writing_submission::workspace::reconcile_added::reconcile_with_index(
                owner,
                scope,
                &path,
                added_only,
                |path| source.capture_index(path),
            )
        })
        .await
    }
    async fn with_source<T, F>(
        &self,
        store: AppStore,
        source: WorkspaceSource,
        retention: WorkerRetention,
        attempt: F,
    ) -> Result<T, Error>
    where
        T: Send + 'static,
        F: Fn(&AppStore, &mut WorkspaceSource) -> Result<T, Flow<Error, ContextNeed>>
            + Clone
            + Send
            + 'static,
    {
        self.with_mapped_source(store, source, retention, attempt, |_, cause| cause)
            .await
    }
    async fn with_mapped_source<T, E, F>(
        &self,
        store: AppStore,
        mut source: WorkspaceSource,
        retention: WorkerRetention,
        attempt: F,
        map_source_error: fn(Option<&WorkspaceSource>, Error) -> E,
    ) -> Result<T, E>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Fn(&AppStore, &mut WorkspaceSource) -> Result<T, Flow<E, ContextNeed>>
            + Clone
            + Send
            + 'static,
    {
        tokio::spawn(async move {
            loop {
                let owner = store.clone();
                let attempt = attempt.clone();
                let (next, result) = retention
                    .spawn(move || {
                        source.begin();
                        let result = attempt(&owner, &mut source);
                        (source, result)
                    })
                    .await
                    .map_err(|_| map_source_error(None, Error::RecoveryRequired))?;
                source = next;
                match result {
                    Ok(value) => return Ok(value),
                    Err(Flow::Need(input)) => source
                        .resolve(input)
                        .await
                        .map_err(|cause| map_source_error(Some(&source), cause))?,
                    Err(Flow::Fail(cause)) => return Err(cause.error),
                }
            }
        })
        .await
        .map_err(|_| map_source_error(None, Error::RecoveryRequired))?
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "workspace/tests.rs"]
mod tests;

#[path = "workspace/undo.rs"]
mod undo;

#[path = "workspace/compensation.rs"]
mod compensation;

#[path = "workspace/editor_restoration.rs"]
mod editor_restoration;

#[path = "workspace/terminal.rs"]
#[cfg(target_os = "macos")]
mod terminal;
