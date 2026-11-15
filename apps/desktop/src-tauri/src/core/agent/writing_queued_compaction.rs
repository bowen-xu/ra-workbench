//! Typed queue-command preparation, discovery and terminal acknowledgement.
use super::{
    model::AgentModelResolver,
    repository::input_queue::queued_compaction::{
        PrepareIntent, QueuedCompactionRead, QueuedCompactionState, ReadIntent, SettleIntent,
    },
    writing_commands::{PrepareRequest, PreviewError, SubmissionPreview},
    writing_queue::{CommandError, QueueResponse, response},
    writing_queue_preview::QueuePreviewResponse,
    writing_submission::{
        WritingSubmissionError, WritingSubmissionRequest, WritingSubmissionService,
    },
};
use crate::data_interface::SqlImmediateFailure;
use research_agent_core::{conversation::CatalogError, protocol::TurnOutcome};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Prepare {
    pub input_id: Uuid,
    pub expected_queue_revision: u64,
    #[serde(deserialize_with = "required_request")]
    pub previous_request_id: Option<Uuid>,
    pub submission: PrepareRequest,
    pub expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Read {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    #[serde(deserialize_with = "required_request")]
    pub request_id: Option<Uuid>,
}
fn required_request<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<Uuid>, D::Error> {
    Option::<Uuid>::deserialize(decoder)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settle {
    pub operation_id: Uuid,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub input_id: Uuid,
    pub request_id: Uuid,
    pub expected_queue_revision: u64,
    pub claim_revision: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    schema_version: u8,
    kind: &'static str,
    input_id: Uuid,
    request_id: Uuid,
    claim_revision: u64,
    expected_revision: u64,
    preview: SubmissionPreview,
    queue: QueueResponse,
    outcome: Option<TurnOutcome>,
    current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    retirement: Option<Box<super::writing_queue_preview::CancelStatusResponse>>,
}
#[derive(Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum Discovery {
    Compact(Preview),
    Submission {
        schema_version: u8,
        kind: &'static str,
        preview: QueuePreviewResponse,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settled {
    schema_version: u8,
    input_id: Uuid,
    request_id: Uuid,
    queue: QueueResponse,
    outcome: TurnOutcome,
}
#[derive(Debug, Serialize)]
#[serde(tag = "code", content = "reason", rename_all = "camelCase")]
pub enum Error {
    Invalid,
    Preparation(PreviewError),
    Queue(CommandError),
    Storage(Box<StorageFailure>),
    #[serde(untagged)]
    Sourced(Box<super::writing_submission::WritingFailure<Error>>),
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageFailure {
    conversation_id: Uuid,
    request_id: Option<Uuid>,
    operation_id: Uuid,
    #[serde(flatten)]
    cause: SqlImmediateFailure,
}
impl Error {
    pub(crate) fn source(error: WritingSubmissionError, intent: &PrepareIntent) -> Self {
        use super::writing_submission::WritingFailure;
        let (error, source) = error.split_source();
        let error = match error {
            WritingSubmissionError::Storage(cause) => Self::data(
                cause.into_inner(),
                intent.conversation_id,
                Some(intent.request_id),
                intent.request_id,
            ),
            cause => Self::preparation(cause),
        };
        match source {
            Some(source) => Self::Sourced(Box::new(WritingFailure::new(
                error,
                source,
                intent.conversation_id,
                intent.manuscript_id,
                intent.request_id,
                intent.input_id,
            ))),
            None => error,
        }
    }
    pub(crate) fn preparation(error: WritingSubmissionError) -> Self {
        Self::Preparation(error.into())
    }
    pub(crate) fn data(
        error: SqlImmediateFailure,
        conversation_id: Uuid,
        request_id: Option<Uuid>,
        operation_id: Uuid,
    ) -> Self {
        match error {
            SqlImmediateFailure::Domain(error) => Self::Queue(match error {
                CatalogError::InvalidInput => CommandError::Invalid,
                CatalogError::Conflict => CommandError::Conflict,
                CatalogError::Busy => CommandError::Busy,
                CatalogError::NotFound => CommandError::Closed,
                CatalogError::TooLarge => CommandError::Capacity,
                CatalogError::Corrupt => CommandError::Corrupt,
                CatalogError::Unavailable => CommandError::Unavailable,
            }),
            error => Self::Storage(Box::new(StorageFailure {
                conversation_id,
                request_id,
                operation_id,
                cause: error,
            })),
        }
    }
}

fn preview(state: QueuedCompactionState, manuscript: Uuid, input: Uuid) -> Result<Preview, Error> {
    let retirement = state
        .retirement
        .map(|value| {
            super::writing_queue_preview::CancelStatusResponse::committed(*value).map(Box::new)
        })
        .transpose()
        .map_err(Error::Queue)?;
    Ok(Preview {
        schema_version: 1,
        kind: "compact",
        input_id: input,
        request_id: state.record.request_id,
        claim_revision: state.claim_revision,
        expected_revision: state.expected_revision,
        preview: SubmissionPreview::from_record(state.record, manuscript),
        queue: response(state.queue, manuscript).map_err(Error::Queue)?,
        outcome: state.outcome,
        current: state.current,
        retirement,
    })
}

pub(crate) async fn prepare(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    request: Prepare,
) -> Result<Preview, Error> {
    let intent = PrepareIntent {
        conversation_id: request.submission.conversation_id,
        manuscript_id: request.submission.context.manuscript_id,
        input_id: request.input_id,
        request_id: request.submission.request_id,
        expected_queue_revision: request.expected_queue_revision,
        expected_revision: request.expected_revision,
        previous_request_id: request.previous_request_id,
        request_json: serde_json::to_value(&request).map_err(|_| Error::Invalid)?,
    };
    let manuscript = intent.manuscript_id;
    let input = intent.input_id;
    let source = request.submission;
    let source = WritingSubmissionRequest {
        conversation_id: source.conversation_id,
        request_id: source.request_id,
        text: source.text,
        user_input: source.user_input,
        context: source.context,
        resources: source.resources,
        retrieval: source.retrieval,
        model: source.model.map(Into::into),
        session_config: source.session_config.map(Into::into),
        turn_config: source.turn_config.map(Into::into),
    };
    preview(
        service
            .prepare_queued_compaction(models, intent, source)
            .await?,
        manuscript,
        input,
    )
}

pub(crate) async fn read(
    service: &WritingSubmissionService,
    request: Read,
) -> Result<Option<Discovery>, Error> {
    let manuscript = request.manuscript_id;
    let input = request.input_id;
    let state = service
        .read_queued_compaction(ReadIntent {
            conversation_id: request.conversation_id,
            manuscript_id: manuscript,
            input_id: input,
            request_id: request.request_id,
        })
        .await?;
    state
        .map(|state| match state {
            QueuedCompactionRead::Compact(state) => {
                preview(state, manuscript, input).map(Discovery::Compact)
            }
            QueuedCompactionRead::Submission { record, queue } => Ok(Discovery::Submission {
                schema_version: 1,
                kind: "submission",
                preview: QueuePreviewResponse {
                    schema_version: 1,
                    input_id: input,
                    preview: SubmissionPreview::from_record(record, manuscript),
                    queue: response(queue, manuscript).map_err(Error::Queue)?,
                },
            }),
        })
        .transpose()
}
pub(crate) async fn settle(
    service: &WritingSubmissionService,
    request: Settle,
) -> Result<Settled, Error> {
    let input = request.input_id;
    let original = request.request_id;
    let manuscript = request.manuscript_id;
    let state = service
        .settle_queued_compaction(SettleIntent {
            operation_id: request.operation_id,
            conversation_id: request.conversation_id,
            manuscript_id: manuscript,
            input_id: input,
            request_id: original,
            expected_queue_revision: request.expected_queue_revision,
            claim_revision: request.claim_revision,
        })
        .await?;
    Ok(Settled {
        schema_version: 1,
        input_id: input,
        request_id: original,
        queue: response(state.queue, manuscript).map_err(Error::Queue)?,
        outcome: state.outcome,
    })
}

#[tauri::command]
pub(crate) async fn agent_prepare_writing_queued_compaction(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    request: Prepare,
) -> Result<Preview, Error> {
    prepare(&service, &models, request).await
}
#[tauri::command]
pub(crate) async fn agent_read_writing_queued_compaction(
    service: State<'_, WritingSubmissionService>,
    request: Read,
) -> Result<Option<Discovery>, Error> {
    read(&service, request).await
}
#[tauri::command]
pub(crate) async fn agent_settle_writing_queued_compaction(
    service: State<'_, WritingSubmissionService>,
    request: Settle,
) -> Result<Settled, Error> {
    settle(&service, request).await
}

#[cfg(test)]
#[path = "writing_queued_compaction/tests.rs"]
mod tests;
