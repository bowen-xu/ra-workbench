use super::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

const KIND: &str = "agent.queue.preview.cancel";

#[derive(Clone, Debug)]
pub(crate) struct CancelPreviewIntent {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub request_id: Uuid,
    pub expected_queue_revision: u64,
    pub input_id: Uuid,
    pub preview_request_id: Uuid,
}
#[derive(Debug)]
pub(crate) enum QueuePreviewCancelStatus {
    Committed {
        queue: InputQueue,
        receipt: Box<SqlImmediateReceipt>,
    },
    NotFound,
    Unconfirmed(Box<SqlImmediateDiagnostic>),
}

#[derive(Clone, Debug)]
pub(crate) struct QueuePreviewRetirement {
    pub intent: CancelPreviewIntent,
    pub queue: InputQueue,
    pub receipt: SqlImmediateReceipt,
}

pub(super) fn identity(
    command: &CancelPreviewIntent,
) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    if [
        command.conversation_id,
        command.manuscript_id,
        command.request_id,
        command.input_id,
        command.preview_request_id,
    ]
    .iter()
    .any(Uuid::is_nil)
        || command.expected_queue_revision >= 9_007_199_254_740_991
    {
        return Err(SqlImmediateFailure::Invalid(Box::new(
            SqlImmediateDiagnostic::for_operation(
                command.request_id,
                None,
                "validate",
                "invalid_queue_cancel_identity",
            ),
        )));
    }
    #[derive(Serialize)]
    struct Frozen {
        kind: &'static str,
        version: u32,
        operation: Uuid,
        conversation: Uuid,
        manuscript: Uuid,
        input: Uuid,
        preview: Uuid,
        queue_revision: u64,
    }
    let bytes = serde_json::to_vec(&Frozen {
        kind: KIND,
        version: 1,
        operation: command.request_id,
        conversation: command.conversation_id,
        manuscript: command.manuscript_id,
        input: command.input_id,
        preview: command.preview_request_id,
        queue_revision: command.expected_queue_revision,
    })
    .map_err(|_| {
        SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
            command.request_id,
            None,
            "validate",
            "queue_cancel_encoding_failed",
        )))
    })?;
    Ok(ReceiptIdentity {
        operation_id: command.request_id,
        kind: KIND,
        version: 1,
        fingerprint: Sha256::digest(bytes).into(),
    })
}
