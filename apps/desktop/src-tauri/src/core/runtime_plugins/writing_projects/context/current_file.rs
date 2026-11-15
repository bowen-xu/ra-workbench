//! Finite manuscript reads consume the original owned preparation worker loop.
use super::Binding;
use crate::agent::writing_context::WritingContextReader;
use crate::{
    agent::{
        repository::WorkerRetention,
        writing_context::{ContextNeed, WritingContextError},
        writing_submission::WritingSubmissionError,
    },
    ai::structured_resource_validation::Flow,
    storage::AppStore,
};
use rusqlite::Connection;
use uuid::Uuid;

impl Binding {
    pub(crate) async fn read_context<T, F>(
        &self,
        store: &AppStore,
        operation: Uuid,
        mode: crate::data_interface::ContextReadMode,
        read: F,
    ) -> Result<T, WritingSubmissionError>
    where
        T: Send + 'static,
        F: Fn(
                &mut Connection,
                &mut dyn WritingContextReader,
            ) -> Result<T, Flow<WritingContextError, ContextNeed>>
            + Send
            + Sync
            + 'static,
    {
        self.run(
            store.shared_agent_connection(),
            operation,
            WorkerRetention::default(),
            // Directory authorization belongs to the caller's existing scope.
            // This reader consumes only individual current-file decisions.
            false,
            move |db, reader| {
                db.read_writing_context(operation, mode, reader, |sql, reader| read(sql, reader))
            },
        )
        .await
    }
}
