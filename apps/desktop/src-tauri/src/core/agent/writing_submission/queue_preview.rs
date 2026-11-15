//! Queued previews use the same native model resolver and source materialization.
use super::*;
use crate::agent::repository::input_queue;
use research_agent_core::input_queue::InputQueue;
#[path = "queue_preview/cancel.rs"]
mod cancel;
#[path = "queue_preview/prepare.rs"]
mod prepare;

#[derive(Debug, thiserror::Error)]
pub enum WritingQueuePreviewError {
    #[error(transparent)]
    Writing(#[from] WritingSubmissionError),
    #[error(transparent)]
    Storage(#[from] crate::data_interface::SqlImmediateFailure),
}
pub(crate) use crate::data_interface::QueuePreviewPrepared as PreparedWritingQueuePreview;
impl WritingSubmissionService {
    pub(crate) async fn inspect_queued_start(
        &self,
        intent: input_queue::start_inspection::QueueStartIntent,
    ) -> crate::data_interface::QueueStartInspection {
        let operation = intent.request_id;
        let database = self.store.shared_agent_connection();
        self.worker_retention
            .spawn(move || database.inspect_queue_start(&intent))
            .await
            .unwrap_or_else(|_| {
                crate::data_interface::QueueStartInspection::Unconfirmed(Box::new(
                    crate::data_interface::SqlImmediateDiagnostic {
                        operation_id: operation,
                        epoch: None,
                        sql_fence: None,
                        phase: "queue-start-status",
                        sqlite_code: None,
                        sqlite_extended_code: None,
                        safe_cause: "queue_start_inspection_worker_unavailable",
                    },
                ))
            })
    }
    pub async fn pause_queue_for_turn(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<bool, input_queue::Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store
                .connection()
                .map_err(|_| input_queue::Error::Storage)?;
            input_queue::pause_for_turn_checked(&mut db, conversation, turn)
        })
        .await
        .map_err(|_| input_queue::Error::Storage)?
    }

    pub async fn list_unstarted_queue_admissions(
        &self,
        manuscript: Uuid,
        conversation: Uuid,
        after: Option<Uuid>,
        limit: u8,
    ) -> Result<Vec<input_queue::UnstartedQueueAdmission>, input_queue::Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store
                .connection()
                .map_err(|_| input_queue::Error::Storage)?;
            input_queue::list_unstarted_checked(&mut db, conversation, after, limit, |db| {
                check_binding(db, conversation, manuscript).map_err(|error| match error {
                    WritingSubmissionError::Unbound => input_queue::Error::Closed,
                    _ => input_queue::Error::Storage,
                })
            })
        })
        .await
        .map_err(|_| input_queue::Error::Storage)?
    }

    pub async fn read_pending_queue_preview(
        &self,
        manuscript: Uuid,
        conversation: Uuid,
    ) -> Result<Option<(SubmissionRecord, InputQueue)>, input_queue::Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store
                .connection()
                .map_err(|_| input_queue::Error::Storage)?;
            input_queue::read_pending_preview_checked(&mut db, conversation, |db| {
                check_binding(db, conversation, manuscript).map_err(|error| match error {
                    WritingSubmissionError::Unbound => input_queue::Error::Closed,
                    _ => input_queue::Error::Storage,
                })
            })
        })
        .await
        .map_err(|_| input_queue::Error::Storage)?
    }
}

#[cfg(test)]
#[path = "queue_preview/tests.rs"]
mod tests;
