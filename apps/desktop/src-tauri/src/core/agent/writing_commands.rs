//! Local submission preview and receipt recovery. Neither command starts a turn.
use super::{
    WritingContextRequest,
    model::AgentModelResolver,
    retrieval_context::RetrievalSelection,
    writing_submission::{
        PreparedWritingSubmission, WritingSubmissionError, WritingSubmissionReceipt,
        WritingSubmissionRequest, WritingSubmissionService,
    },
};
use research_agent_core::submission::{SubmissionError, SubmissionInput};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
#[path = "writing_commands/prepare_status.rs"]
mod prepare_status;
pub use prepare_status::{PrepareCommandError, PrepareStatusResponse};
pub(crate) use prepare_status::{PrepareStatus, prepare_status, status_value};

pub use super::config::ConfigOverride;

pub use super::model::ModelSelection;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
    pub context: WritingContextRequest,
    #[serde(default)]
    pub resources: Vec<super::context_session::sources::Resource>,
    #[serde(default)]
    pub retrieval: Option<RetrievalSelection>,
    pub model: Option<ModelSelection>,
    #[serde(default)]
    pub session_config: Option<ConfigOverride>,
    #[serde(default)]
    pub turn_config: Option<ConfigOverride>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub manuscript_id: Uuid,
}

/// The full frozen input is returned for disclosure review, without credentials,
/// provider configuration, model internals, or permission to execute tools.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionPreview {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub manuscript_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub input: SubmissionInput,
}
impl SubmissionPreview {
    pub(crate) fn from_prepared(value: PreparedWritingSubmission, manuscript_id: Uuid) -> Self {
        Self::from_record(value.record, manuscript_id)
    }
    pub(crate) fn from_record(
        record: research_agent_core::submission::SubmissionRecord,
        manuscript_id: Uuid,
    ) -> Self {
        Self {
            schema_version: 1,
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            manuscript_id,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            input: record.input,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewError {
    Unbound,
    InvalidInput,
    Conflict,
    NotFound,
    ContextUnavailable,
    ContextBudget,
    ModelUnavailable,
    Unavailable,
}
impl From<WritingSubmissionError> for PreviewError {
    fn from(value: WritingSubmissionError) -> Self {
        use super::WritingContextError;
        match value {
            WritingSubmissionError::Unbound => Self::Unbound,
            WritingSubmissionError::Input(SubmissionError::InvalidInput)
            | WritingSubmissionError::Context(WritingContextError::InvalidInput) => {
                Self::InvalidInput
            }
            WritingSubmissionError::Input(SubmissionError::Conflict)
            | WritingSubmissionError::Context(WritingContextError::Conflict) => Self::Conflict,
            WritingSubmissionError::Input(SubmissionError::NotFound) => Self::NotFound,
            WritingSubmissionError::Context(WritingContextError::Budget) => Self::ContextBudget,
            WritingSubmissionError::Context(WritingContextError::Unavailable) => {
                Self::ContextUnavailable
            }
            WritingSubmissionError::Model(_) => Self::ModelUnavailable,
            _ => Self::Unavailable,
        }
    }
}

#[tauri::command]
pub async fn agent_prepare_writing_submission(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
) -> Result<SubmissionPreview, PrepareCommandError> {
    prepare(&service, &models, request).await
}

pub(crate) async fn prepare(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    request: PrepareRequest,
) -> Result<SubmissionPreview, PrepareCommandError> {
    let manuscript_id = request.context.manuscript_id;
    let prepared = service
        .prepare_preview(models, request.clone().into(), None)
        .await
        .map_err(|cause| request.preparation_failure(cause))?;
    Ok(SubmissionPreview::from_record(prepared, manuscript_id))
}

#[tauri::command]
pub async fn agent_read_writing_submission_prepare_status(
    service: State<'_, WritingSubmissionService>,
    request: PrepareRequest,
) -> Result<PrepareStatusResponse, PrepareCommandError> {
    prepare_status(&service, request).await
}

#[tauri::command]
pub async fn agent_restore_writing_submission(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    request: RestoreRequest,
) -> Result<SubmissionPreview, PreviewError> {
    restore(&service, &models, request).await
}

pub(crate) async fn restore(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    request: RestoreRequest,
) -> Result<SubmissionPreview, PreviewError> {
    let prepared = service
        .restore(
            models,
            WritingSubmissionReceipt {
                conversation_id: request.conversation_id,
                request_id: request.request_id,
                manuscript_id: request.manuscript_id,
            },
        )
        .await?;
    Ok(SubmissionPreview::from_prepared(
        prepared,
        request.manuscript_id,
    ))
}
