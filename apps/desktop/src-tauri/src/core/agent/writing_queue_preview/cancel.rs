//! Keep cancellation identity and diagnostics intact across both native transports.
use super::*;
use crate::{
    agent::catalog_commands::CatalogMutationProofDto,
    data_interface::{
        CancelPreviewIntent, QueuePreviewCancelStatus, QueuePreviewRetirement,
        SqlImmediateDiagnostic, SqlImmediateFailure,
    },
};
use research_agent_core::conversation::{CatalogMutationProof, ConversationId};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelRequest {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub request_id: Uuid,
    pub expected_revision: u64,
    pub input_id: Uuid,
    pub preview_request_id: Uuid,
}
impl CancelRequest {
    fn intent(&self) -> CancelPreviewIntent {
        CancelPreviewIntent {
            conversation_id: self.conversation_id,
            manuscript_id: self.manuscript_id,
            request_id: self.request_id,
            expected_queue_revision: self.expected_revision,
            input_id: self.input_id,
            preview_request_id: self.preview_request_id,
        }
    }
    fn error(&self, cause: SqlImmediateFailure) -> QueuePreviewError {
        QueuePreviewError::Storage(Box::new(QueueStorageFailure {
            conversation_id: self.conversation_id,
            manuscript_id: self.manuscript_id,
            input_id: self.input_id,
            preview_request_id: self.preview_request_id,
            operation_id: self.request_id,
            cause,
        }))
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelStatusResponse {
    schema_version: u8,
    intent: CancelRequest,
    status: CancelStatus,
}
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum CancelStatus {
    Committed {
        proof: CatalogMutationProofDto,
        queue: QueueResponse,
    },
    NotFound,
    Unconfirmed {
        detail: Box<SqlImmediateDiagnostic>,
    },
}

impl CancelStatusResponse {
    pub(crate) fn committed(value: QueuePreviewRetirement) -> Result<Self, CommandError> {
        let intent = value.intent;
        Ok(Self {
            schema_version: 1,
            status: CancelStatus::Committed {
                proof: CatalogMutationProofDto::new(
                    CatalogMutationProof {
                        operation_id: value.receipt.operation_id,
                        conversation_id: ConversationId::from_uuid(intent.conversation_id),
                        epoch: value.receipt.epoch,
                        sql_fence: value.receipt.sql_fence.position,
                        replayed: value.receipt.replayed,
                    },
                    "agent.queue.preview.cancel",
                ),
                queue: response(value.queue, intent.manuscript_id)?,
            },
            intent: CancelRequest {
                conversation_id: intent.conversation_id,
                manuscript_id: intent.manuscript_id,
                request_id: intent.request_id,
                expected_revision: intent.expected_queue_revision,
                input_id: intent.input_id,
                preview_request_id: intent.preview_request_id,
            },
        })
    }
}

pub(crate) async fn cancel(
    service: &WritingSubmissionService,
    request: CancelRequest,
) -> Result<QueueResponse, QueuePreviewError> {
    let queue = service
        .cancel_queue_preview(
            request.manuscript_id,
            request.conversation_id,
            request.request_id,
            request.expected_revision,
            request.input_id,
            request.preview_request_id,
        )
        .await
        .map_err(|cause| request.error(cause))?;
    response(queue, request.manuscript_id).map_err(QueuePreviewError::Queue)
}

pub(crate) async fn cancel_status(
    service: &WritingSubmissionService,
    request: CancelRequest,
) -> Result<CancelStatusResponse, QueuePreviewError> {
    let status = match service
        .read_queue_preview_cancel_status(request.intent())
        .await
        .map_err(|cause| request.error(cause))?
    {
        QueuePreviewCancelStatus::Committed { queue, receipt } => {
            return CancelStatusResponse::committed(QueuePreviewRetirement {
                intent: request.intent(),
                queue,
                receipt: *receipt,
            })
            .map_err(QueuePreviewError::Queue);
        }
        QueuePreviewCancelStatus::NotFound => CancelStatus::NotFound,
        QueuePreviewCancelStatus::Unconfirmed(detail) => CancelStatus::Unconfirmed { detail },
    };
    Ok(CancelStatusResponse {
        schema_version: 1,
        intent: request,
        status,
    })
}

#[cfg(test)]
#[path = "cancel/tests.rs"]
mod tests;
