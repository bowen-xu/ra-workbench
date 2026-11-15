//! Freeze the decoded complete request as the identity of one ordinary prepare.
use super::*;
use crate::{
    agent::{
        catalog_commands::CatalogMutationProofDto, writing_submission::WritingQueuePreviewError,
    },
    data_interface::{
        QueuePreviewPrepareIntent, QueuePreviewPrepareStatus, QueuePreviewPrepared,
        SqlImmediateDiagnostic, SqlImmediateFailure,
    },
};
use research_agent_core::conversation::{CatalogMutationProof, ConversationId};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueuePreviewRequest {
    pub input_id: Uuid,
    pub expected_revision: u64,
    pub submission: PrepareRequest,
}
impl QueuePreviewRequest {
    fn intent(&self) -> Result<QueuePreviewPrepareIntent, QueuePreviewError> {
        if self.expected_revision >= 9_007_199_254_740_991 || self.input_id.is_nil() {
            return Err(QueuePreviewError::Invalid);
        }
        Ok(QueuePreviewPrepareIntent {
            conversation_id: self.submission.conversation_id,
            manuscript_id: self.submission.context.manuscript_id,
            input_id: self.input_id,
            request_id: self.submission.request_id,
            expected_queue_revision: self.expected_revision,
            // Serialize after decoding: omitted defaults and explicit defaults
            // name the same intent; selected resources and all options remain.
            request_json: serde_json::to_value(self).map_err(|_| QueuePreviewError::Invalid)?,
        })
    }
    fn source_error(
        &self,
        cause: super::super::writing_submission::WritingSubmissionError,
    ) -> QueuePreviewError {
        use super::super::writing_submission::{WritingFailure, WritingSubmissionError};
        let (cause, source) = cause.split_source();
        let error = match cause {
            WritingSubmissionError::Storage(cause) => self.error(cause.into_inner()),
            cause => QueuePreviewError::Preparation(cause.into()),
        };
        match source {
            Some(source) => QueuePreviewError::Sourced(Box::new(WritingFailure::new(
                error,
                source,
                self.submission.conversation_id,
                self.submission.context.manuscript_id,
                self.submission.request_id,
                self.input_id,
            ))),
            None => error,
        }
    }
    fn error(&self, cause: SqlImmediateFailure) -> QueuePreviewError {
        QueuePreviewError::Storage(Box::new(QueueStorageFailure {
            conversation_id: self.submission.conversation_id,
            manuscript_id: self.submission.context.manuscript_id,
            input_id: self.input_id,
            preview_request_id: self.submission.request_id,
            operation_id: self.submission.request_id,
            cause,
        }))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareStatusResponse {
    schema_version: u8,
    intent: QueuePreviewRequest,
    status: PrepareStatus,
}
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum PrepareStatus {
    Committed {
        proof: CatalogMutationProofDto,
        result: Box<QueuePreviewResponse>,
    },
    NotFound,
    Unconfirmed {
        detail: Box<SqlImmediateDiagnostic>,
    },
}

fn response_from_prepared(
    request: &QueuePreviewRequest,
    state: QueuePreviewPrepared,
) -> Result<QueuePreviewResponse, QueuePreviewError> {
    let manuscript = request.submission.context.manuscript_id;
    Ok(QueuePreviewResponse {
        schema_version: 1,
        input_id: request.input_id,
        preview: SubmissionPreview::from_record(state.record, manuscript),
        queue: response(state.queue, manuscript).map_err(QueuePreviewError::Queue)?,
    })
}

pub(crate) async fn prepare(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    request: QueuePreviewRequest,
) -> Result<QueuePreviewResponse, QueuePreviewError> {
    let intent = request.intent()?;
    let submission = request.submission.clone();
    let state = service
        .prepare_queue_preview(
            models,
            intent,
            WritingSubmissionRequest {
                resources: submission.resources,
                user_input: submission.user_input,
                conversation_id: submission.conversation_id,
                request_id: submission.request_id,
                text: submission.text,
                context: submission.context,
                retrieval: submission.retrieval,
                model: submission.model.map(|model| AgentModelSelection {
                    provider_id: model.provider_id,
                    model_id: model.model_id,
                    reasoning_effort: model.reasoning_effort,
                    service_tier: model.service_tier,
                }),
                session_config: submission.session_config.map(Into::into),
                turn_config: submission.turn_config.map(Into::into),
            },
        )
        .await
        .map_err(|error| match error {
            WritingQueuePreviewError::Writing(cause) => request.source_error(cause),
            WritingQueuePreviewError::Storage(cause) => request.error(cause),
        })?;
    response_from_prepared(&request, state)
}

pub(crate) async fn prepare_status(
    service: &WritingSubmissionService,
    request: QueuePreviewRequest,
) -> Result<PrepareStatusResponse, QueuePreviewError> {
    let status = match service
        .read_queue_preview_prepare_status(request.intent()?)
        .await
        .map_err(|cause| request.error(cause))?
    {
        QueuePreviewPrepareStatus::Committed { state, receipt } => PrepareStatus::Committed {
            proof: CatalogMutationProofDto::new(
                CatalogMutationProof {
                    operation_id: receipt.operation_id,
                    conversation_id: ConversationId::from_uuid(request.submission.conversation_id),
                    epoch: receipt.epoch,
                    sql_fence: receipt.sql_fence.position,
                    replayed: receipt.replayed,
                },
                "agent.queue.preview.prepare",
            ),
            result: Box::new(response_from_prepared(&request, *state)?),
        },
        QueuePreviewPrepareStatus::NotFound => PrepareStatus::NotFound,
        QueuePreviewPrepareStatus::Unconfirmed(detail) => PrepareStatus::Unconfirmed { detail },
    };
    Ok(PrepareStatusResponse {
        schema_version: 1,
        intent: request,
        status,
    })
}

#[cfg(test)]
#[path = "prepare/tests.rs"]
mod tests;
