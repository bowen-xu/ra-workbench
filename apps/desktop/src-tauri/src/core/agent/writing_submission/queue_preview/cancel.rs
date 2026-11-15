//! Ordinary preview cancellation has one typed SQL transaction and original proof.
use super::*;
use crate::data_interface::{
    CancelPreviewIntent, QueuePreviewCancelStatus, SqlImmediateDiagnostic, SqlImmediateFailure,
};

impl WritingSubmissionService {
    pub(crate) async fn cancel_queue_preview(
        &self,
        manuscript: Uuid,
        conversation: Uuid,
        request: Uuid,
        revision: u64,
        input_id: Uuid,
        preview_request: Uuid,
    ) -> Result<InputQueue, SqlImmediateFailure> {
        let intent = CancelPreviewIntent {
            manuscript_id: manuscript,
            conversation_id: conversation,
            request_id: request,
            expected_queue_revision: revision,
            input_id,
            preview_request_id: preview_request,
        };
        let database = self.store.shared_agent_connection();
        self.worker_retention
            .spawn(move || database.cancel_queue_preview(&intent))
            .await
            .map_err(|_| worker_failure(request, true))?
            .map(|(queue, _)| queue)
    }

    pub(crate) async fn read_queue_preview_cancel_status(
        &self,
        intent: CancelPreviewIntent,
    ) -> Result<QueuePreviewCancelStatus, SqlImmediateFailure> {
        let operation = intent.request_id;
        let database = self.store.shared_agent_connection();
        self.worker_retention
            .spawn(move || database.queue_preview_cancel_status(&intent))
            .await
            .map_err(|_| worker_failure(operation, false))?
    }
}

fn worker_failure(operation_id: Uuid, writing: bool) -> SqlImmediateFailure {
    let detail = Box::new(SqlImmediateDiagnostic {
        operation_id,
        epoch: None,
        sql_fence: None,
        phase: if writing {
            "queue-preview-cancel"
        } else {
            "queue-preview-cancel-status"
        },
        sqlite_code: None,
        sqlite_extended_code: None,
        safe_cause: "worker_join_failed",
    });
    if writing {
        SqlImmediateFailure::Unknown(detail)
    } else {
        SqlImmediateFailure::Unavailable(detail)
    }
}
