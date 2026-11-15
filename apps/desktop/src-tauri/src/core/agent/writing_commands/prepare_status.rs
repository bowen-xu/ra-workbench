//! Public preparation facts exclude credentials and never grant execution.
use super::*;
use crate::agent::catalog_commands::CatalogMutationProofDto;
use crate::agent::writing_submission::WritingSourceDiagnostic;
use crate::data_interface::{
    SqlImmediateDiagnostic, SqlImmediateFailure, WRITING_PREPARE_FAMILY, WritingPrepareIntent,
    WritingPrepareStatus,
};
use research_agent_core::{
    conversation::{CatalogMutationProof, ConversationId},
    submission::SubmissionRecord,
};
use serde::ser::SerializeStruct;

#[derive(Debug)]
pub enum PrepareCommandError {
    Native {
        cause: super::super::writing_submission::WritingNativeError,
        conversation_id: Uuid,
        manuscript_id: Uuid,
        operation_id: Uuid,
    },
    Preparation(PreviewError),
    Storage(Box<PrepareStorageFailure>),
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareStorageFailure {
    conversation_id: Uuid,
    manuscript_id: Uuid,
    request_id: Uuid,
    operation_id: Uuid,
    #[serde(flatten)]
    cause: SqlImmediateFailure,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<WritingSourceDiagnostic>,
}

impl Serialize for PrepareCommandError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Native {
                cause,
                conversation_id,
                manuscript_id,
                operation_id,
            } => {
                // The finite Core diagnostic already serializes safe fields.
                // Add the original caller's identities without replacing the
                // Native requestId or emitting duplicate flattened keys.
                let mut value = serde_json::to_value(cause).map_err(serde::ser::Error::custom)?;
                let fields = value.as_object_mut().expect("native error is an object");
                fields.insert("conversationId".into(), serde_json::json!(conversation_id));
                fields.insert("manuscriptId".into(), serde_json::json!(manuscript_id));
                fields.insert("operationId".into(), serde_json::json!(operation_id));
                value.serialize(serializer)
            }
            Self::Preparation(cause) => cause.serialize(serializer),
            Self::Storage(cause) => {
                let mut value = serializer.serialize_struct("PrepareCommandError", 2)?;
                value.serialize_field("code", "storage")?;
                value.serialize_field("reason", cause)?;
                value.end()
            }
        }
    }
}
impl PrepareRequest {
    pub(crate) fn preparation_intent(&self, compact_revision: Option<u64>) -> WritingPrepareIntent {
        WritingPrepareIntent {
            request: self.clone(),
            compact_revision,
        }
    }
    pub(crate) fn preparation_failure(&self, cause: WritingSubmissionError) -> PrepareCommandError {
        match cause {
            WritingSubmissionError::Native(cause) => PrepareCommandError::Native {
                cause,
                conversation_id: self.conversation_id,
                manuscript_id: self.context.manuscript_id,
                operation_id: self.request_id,
            },
            WritingSubmissionError::Storage(cause) => {
                let (cause, source) = cause.into_parts();
                self.storage_failure_with_source(cause, source)
            }
            cause => PrepareCommandError::Preparation(cause.into()),
        }
    }
    pub(crate) fn storage_failure(&self, cause: SqlImmediateFailure) -> PrepareCommandError {
        self.storage_failure_with_source(cause, None)
    }
    fn storage_failure_with_source(
        &self,
        cause: SqlImmediateFailure,
        source: Option<WritingSourceDiagnostic>,
    ) -> PrepareCommandError {
        PrepareCommandError::Storage(Box::new(PrepareStorageFailure {
            conversation_id: self.conversation_id,
            manuscript_id: self.context.manuscript_id,
            request_id: self.request_id,
            operation_id: self.request_id,
            cause,
            source,
        }))
    }
}
impl From<PrepareRequest> for WritingSubmissionRequest {
    fn from(value: PrepareRequest) -> Self {
        Self {
            conversation_id: value.conversation_id,
            request_id: value.request_id,
            text: value.text,
            user_input: value.user_input,
            context: value.context,
            resources: value.resources,
            retrieval: value.retrieval,
            model: value.model.map(Into::into),
            session_config: value.session_config.map(Into::into),
            turn_config: value.turn_config.map(Into::into),
        }
    }
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PrepareStatus<T> {
    Committed {
        proof: CatalogMutationProofDto,
        result: Box<T>,
    },
    NotFound,
    Unconfirmed {
        detail: Box<SqlImmediateDiagnostic>,
    },
}
pub(crate) fn status_value<T>(
    request: &PrepareRequest,
    value: WritingPrepareStatus,
    encode: impl FnOnce(SubmissionRecord) -> T,
) -> PrepareStatus<T> {
    match value {
        WritingPrepareStatus::Committed { record, receipt } => PrepareStatus::Committed {
            proof: CatalogMutationProofDto::new(
                CatalogMutationProof {
                    operation_id: receipt.operation_id,
                    conversation_id: ConversationId::from_uuid(request.conversation_id),
                    epoch: receipt.epoch,
                    sql_fence: receipt.sql_fence.position,
                    replayed: receipt.replayed,
                },
                WRITING_PREPARE_FAMILY,
            ),
            result: Box::new(encode(*record)),
        },
        WritingPrepareStatus::NotFound => PrepareStatus::NotFound,
        WritingPrepareStatus::Unconfirmed(detail) => PrepareStatus::Unconfirmed { detail },
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareStatusResponse {
    schema_version: u8,
    intent: PrepareRequest,
    status: PrepareStatus<SubmissionPreview>,
}
pub(crate) async fn prepare_status(
    service: &WritingSubmissionService,
    request: PrepareRequest,
) -> Result<PrepareStatusResponse, PrepareCommandError> {
    let value = service
        .read_prepare_status(request.preparation_intent(None))
        .await
        .map_err(|cause| request.storage_failure(cause))?;
    let status = status_value(&request, value, |record| {
        SubmissionPreview::from_record(record, request.context.manuscript_id)
    });
    Ok(PrepareStatusResponse {
        schema_version: 1,
        intent: request,
        status,
    })
}

#[cfg(test)]
#[path = "prepare_status/tests.rs"]
mod tests;
