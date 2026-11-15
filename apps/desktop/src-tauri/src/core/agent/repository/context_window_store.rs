//! Existing SQLite blocking executor adapter for the shared checkpoint port.
use super::context_windows::{self, Error, Installation};
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::context::{
    checkpoint::ContextCheckpoint,
    checkpoint_store::{
        CheckpointInstallation, CheckpointStoreError, CheckpointStoreFuture, ContextCheckpointStore,
    },
};
use research_agent_core::{model::ModelError, runner::RunError};
use uuid::Uuid;
#[path = "context_window_store/initial_metadata.rs"]
mod initial_metadata;

pub struct SqliteContextCheckpointStore {
    connection: SharedDatabase,
}
impl SqliteContextCheckpointStore {
    pub(crate) async fn context_fragments(
        &self,
        conversation: Uuid,
        expected: Option<Uuid>,
        after: u64,
    ) -> Result<Vec<super::context_fragments::FragmentBatch>, RunError> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = connection.lock().map_err(|_| RunError::EventSink)?;
            let identity = context_windows::identity::read_or_create_managed(
                &mut db,
                conversation,
                Uuid::now_v7(),
            )
            .map_err(|error| match error {
                context_windows::identity::ManagedIdentityError::Persistence(detail) => {
                    RunError::from(ModelError::PersistenceDiagnostic(detail))
                }
                context_windows::identity::ManagedIdentityError::Domain(_) => RunError::EventSink,
            })?;
            if expected.is_some_and(|id| identity.current != id)
                || (expected.is_none() && identity.number != 0)
            {
                return Err(RunError::EventSink);
            }
            super::context_fragments::read(&db, conversation, identity.current, after)
                .map_err(|_| RunError::EventSink)
        })
        .await
        .map_err(|_| RunError::EventSink)?
    }
    pub(crate) async fn record_context_fragments(
        &self,
        batch: super::context_fragments::FragmentBatch,
    ) -> Result<(), CheckpointStoreError> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = connection
                .lock()
                .map_err(|_| CheckpointStoreError::Unavailable)?;
            super::context_fragments::managed::record(&mut db, &batch).map(|_| ())
        })
        .await
        .map_err(|_| CheckpointStoreError::Unavailable)?
    }
    pub(crate) fn new(connection: impl IntoDatabase) -> Self {
        Self {
            connection: connection.into_database(),
        }
    }
    pub(crate) async fn window_identity(
        &self,
        conversation: Uuid,
    ) -> Result<context_windows::identity::WindowIdentity, CheckpointStoreError> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = connection
                .lock()
                .map_err(|_| CheckpointStoreError::Unavailable)?;
            context_windows::identity::read_or_create_managed(&mut db, conversation, Uuid::now_v7())
                .map_err(|error| match error {
                    context_windows::identity::ManagedIdentityError::Domain(error) => {
                        map_error(error)
                    }
                    context_windows::identity::ManagedIdentityError::Persistence(detail) => {
                        CheckpointStoreError::PersistenceDiagnostic(detail)
                    }
                })
        })
        .await
        .map_err(|_| CheckpointStoreError::Unavailable)?
    }
}
fn map_error(error: Error) -> CheckpointStoreError {
    match error {
        Error::Storage => CheckpointStoreError::Unavailable,
        Error::Invalid => CheckpointStoreError::Invalid,
        Error::Conflict => CheckpointStoreError::Conflict,
        Error::Inactive => CheckpointStoreError::Inactive,
        Error::Canceled => CheckpointStoreError::Canceled,
    }
}
impl ContextCheckpointStore for SqliteContextCheckpointStore {
    fn take_pending_session_start(
        &self,
        conversation: Uuid,
    ) -> CheckpointStoreFuture<'_, Option<ContextCheckpoint>> {
        let connection = self.connection.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut db = connection
                    .lock()
                    .map_err(|_| CheckpointStoreError::Unavailable)?;
                context_windows::session_start::take_managed(&mut db, conversation)
            })
            .await
            .map_err(|_| CheckpointStoreError::Unavailable)?
        })
    }
    fn read_active(
        &self,
        conversation: Uuid,
    ) -> CheckpointStoreFuture<'_, Option<ContextCheckpoint>> {
        let connection = self.connection.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let db = connection
                    .lock()
                    .map_err(|_| CheckpointStoreError::Unavailable)?;
                context_windows::read_active(&db, conversation).map_err(map_error)
            })
            .await
            .map_err(|_| CheckpointStoreError::Unavailable)?
        })
    }
    fn install_at(
        &self,
        checkpoint: ContextCheckpoint,
        expected_revision: u64,
    ) -> CheckpointStoreFuture<'_, CheckpointInstallation> {
        self.install_owned(checkpoint, expected_revision, None)
    }
}
impl SqliteContextCheckpointStore {
    pub(crate) fn install_owned(
        &self,
        checkpoint: ContextCheckpoint,
        expected_revision: u64,
        cancellation: Option<crate::agent::cancellation::AgentCancellation>,
    ) -> CheckpointStoreFuture<'_, CheckpointInstallation> {
        let connection = self.connection.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut db = connection
                    .lock()
                    .map_err(|_| CheckpointStoreError::Unavailable)?;
                context_windows::install_managed::install(
                    &mut db,
                    &checkpoint,
                    expected_revision,
                    cancellation.as_ref(),
                )
                .map(|result| match result {
                    Installation::Installed => CheckpointInstallation::Installed,
                    Installation::AlreadyInstalled { active } => {
                        CheckpointInstallation::AlreadyInstalled { active }
                    }
                })
            })
            .await
            .map_err(|_| CheckpointStoreError::Unavailable)?
        })
    }
}
