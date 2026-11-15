//! Host-neutral transport glue over the existing typed SQL owner and worker.
use super::{CommandError, EditStatusResponse};
use crate::{
    agent::repository::WorkerRetention,
    data_interface::{
        QueueEditFailure, QueueEditIntent, SqlImmediateDiagnostic, SqlImmediateFailure,
    },
    storage::AppStore,
};
use research_agent_core::input_queue::InputQueue;

pub(crate) fn worker_failure(intent: &QueueEditIntent) -> QueueEditFailure {
    QueueEditFailure::Storage(SqlImmediateFailure::Unknown(Box::new(
        SqlImmediateDiagnostic {
            operation_id: intent.request_id,
            epoch: None,
            sql_fence: None,
            phase: "queue-edit-worker",
            sqlite_code: None,
            sqlite_extended_code: None,
            safe_cause: "queue_edit_worker_failed",
        },
    )))
}
pub(crate) async fn execute_edit(
    store: &AppStore,
    intent: QueueEditIntent,
    retention: WorkerRetention,
) -> Result<InputQueue, CommandError> {
    let database = store.shared_agent_connection();
    let original = intent.clone();
    retention
        .spawn(move || {
            database
                .edit_input_queue(&intent)
                .map(|outcome| outcome.into_queue())
                .map_err(|cause| super::failure(&intent, cause))
        })
        .await
        .map_err(|_| super::failure(&original, worker_failure(&original)))?
}
pub(crate) async fn read_edit_status(
    store: &AppStore,
    intent: QueueEditIntent,
    retention: WorkerRetention,
) -> Result<EditStatusResponse, CommandError> {
    let database = store.shared_agent_connection();
    let original = intent.clone();
    retention
        .spawn(move || {
            let status = database
                .input_queue_edit_status(&intent)
                .map_err(|cause| super::failure(&intent, cause))?;
            super::edit_status(intent, status)
        })
        .await
        .map_err(|_| super::failure(&original, worker_failure(&original)))?
}
