//! Frozen start inspection is evidence only; it never admits or resumes a task.
use super::*;
use crate::{
    agent::{
        repository::input_queue::start_inspection::{Fact, QueueStartIntent},
        writing_commands::SubmissionPreview,
    },
    data_interface::{QueueStartInspection, SqlImmediateDiagnostic},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedStartStatusResponse {
    schema_version: u8,
    intent: QueuedStartRequest,
    status: Status,
}
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Status {
    Prepared { preview: Box<SubmissionPreview> },
    Admitted { preview: Box<SubmissionPreview> },
    Started { result: StartResponse },
    NotFound,
    Unconfirmed { detail: Box<SqlImmediateDiagnostic> },
}
impl QueuedStartRequest {
    fn inspection(&self) -> Result<QueueStartIntent, StartError> {
        if [
            self.input_id,
            self.start.conversation_id,
            self.start.manuscript_id,
            self.start.request_id,
            self.start.context_snapshot_id,
        ]
        .iter()
        .any(Uuid::is_nil)
            || self.expected_revision >= 9_007_199_254_740_991
        {
            return Err(StartError::ConfirmationMismatch);
        }
        Ok(QueueStartIntent {
            input_id: self.input_id,
            expected_revision: self.expected_revision,
            conversation_id: self.start.conversation_id,
            manuscript_id: self.start.manuscript_id,
            request_id: self.start.request_id,
            context_snapshot_id: self.start.context_snapshot_id,
            model: self.start.model.clone(),
            mode: self.start.mode,
        })
    }
}
pub(crate) async fn read_status(
    service: &WritingSubmissionService,
    request: QueuedStartRequest,
) -> Result<QueuedStartStatusResponse, StartError> {
    let identity = request.inspection()?;
    let status = match service.inspect_queued_start(identity).await {
        QueueStartInspection::Confirmed(Fact::Prepared(record)) => Status::Prepared {
            preview: Box::new(SubmissionPreview::from_record(
                *record,
                request.start.manuscript_id,
            )),
        },
        QueueStartInspection::Confirmed(Fact::Admitted(record)) => Status::Admitted {
            preview: Box::new(SubmissionPreview::from_record(
                *record,
                request.start.manuscript_id,
            )),
        },
        QueueStartInspection::Confirmed(Fact::Started { turn_id, outcome }) => Status::Started {
            result: StartResponse {
                schema_version: 1,
                conversation_id: request.start.conversation_id,
                request_id: request.start.request_id,
                turn_id,
                run_id: None,
                already_started: true,
                outcome,
            },
        },
        QueueStartInspection::Confirmed(Fact::Missing) => Status::NotFound,
        QueueStartInspection::Unconfirmed(detail) => Status::Unconfirmed { detail },
    };
    Ok(QueuedStartStatusResponse {
        schema_version: 1,
        intent: request,
        status,
    })
}
