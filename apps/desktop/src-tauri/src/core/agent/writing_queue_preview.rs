//! Queue preview transport, without model execution or serialized credentials.
use super::{
    model::{AgentModelResolver, AgentModelSelection},
    writing_commands::{PrepareRequest, PreviewError, SubmissionPreview},
    writing_queue::{CommandError, QueueResponse, response},
    writing_submission::{WritingSubmissionRequest, WritingSubmissionService},
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
#[path = "writing_queue_preview/cancel.rs"]
mod cancel;
#[path = "writing_queue_preview/prepare.rs"]
mod prepare;
pub use cancel::{CancelRequest, CancelStatusResponse};
pub(crate) use cancel::{cancel, cancel_status};
pub use prepare::{PrepareStatusResponse, QueuePreviewRequest};
pub(crate) use prepare::{prepare, prepare_status};

#[tauri::command]
pub async fn agent_cancel_writing_queue_preview(
    service: State<'_, WritingSubmissionService>,
    request: CancelRequest,
) -> Result<QueueResponse, QueuePreviewError> {
    cancel(&service, request).await
}
#[tauri::command]
pub async fn agent_read_writing_queue_preview_cancel_status(
    service: State<'_, WritingSubmissionService>,
    request: CancelRequest,
) -> Result<CancelStatusResponse, QueuePreviewError> {
    cancel_status(&service, request).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePreviewResponse {
    pub schema_version: u8,
    pub input_id: Uuid,
    pub preview: SubmissionPreview,
    pub queue: QueueResponse,
}
#[derive(Debug, Serialize)]
#[serde(tag = "code", content = "reason", rename_all = "camelCase")]
pub enum QueuePreviewError {
    Invalid,
    Preparation(PreviewError),
    Queue(CommandError),
    Storage(Box<QueueStorageFailure>),
    #[serde(untagged)]
    Sourced(Box<super::writing_submission::WritingFailure<QueuePreviewError>>),
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStorageFailure {
    conversation_id: Uuid,
    manuscript_id: Uuid,
    input_id: Uuid,
    preview_request_id: Uuid,
    operation_id: Uuid,
    #[serde(flatten)]
    cause: crate::data_interface::SqlImmediateFailure,
}
#[tauri::command]
pub async fn agent_read_pending_writing_queue_preview(
    service: State<'_, WritingSubmissionService>,
    request: super::writing_queue::ReadRequest,
) -> Result<Option<QueuePreviewResponse>, QueuePreviewError> {
    read_pending(&service, request).await
}
pub(crate) async fn read_pending(
    service: &WritingSubmissionService,
    request: super::writing_queue::ReadRequest,
) -> Result<Option<QueuePreviewResponse>, QueuePreviewError> {
    if request.manuscript_id.is_nil() || request.conversation_id.is_nil() {
        return Err(QueuePreviewError::Invalid);
    }
    let Some((record, queue)) = service
        .read_pending_queue_preview(request.manuscript_id, request.conversation_id)
        .await
        .map_err(|error| QueuePreviewError::Queue(error.into()))?
    else {
        return Ok(None);
    };
    let input_id = queue
        .preparing()
        .ok_or(QueuePreviewError::Queue(CommandError::Corrupt))?;
    Ok(Some(QueuePreviewResponse {
        schema_version: 1,
        input_id,
        preview: SubmissionPreview::from_record(record, request.manuscript_id),
        queue: response(queue, request.manuscript_id).map_err(QueuePreviewError::Queue)?,
    }))
}
#[tauri::command]
pub async fn agent_prepare_writing_queue_preview(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    request: QueuePreviewRequest,
) -> Result<QueuePreviewResponse, QueuePreviewError> {
    prepare(&service, &models, request).await
}
#[tauri::command]
pub async fn agent_read_writing_queue_preview_prepare_status(
    service: State<'_, WritingSubmissionService>,
    request: QueuePreviewRequest,
) -> Result<PrepareStatusResponse, QueuePreviewError> {
    prepare_status(&service, request).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_wire_cannot_change_model_scope_or_start_execution() {
        let base = serde_json::json!({"conversationId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"expectedRevision":2,"inputId":Uuid::new_v4(),"previewRequestId":Uuid::new_v4()});
        assert!(serde_json::from_value::<CancelRequest>(base.clone()).is_ok());
        for field in ["model", "text", "execute", "context"] {
            let mut value = base.clone();
            value[field] = serde_json::json!("unexpected");
            assert!(serde_json::from_value::<CancelRequest>(value).is_err());
        }
    }
}
