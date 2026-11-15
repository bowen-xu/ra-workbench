//! Writing compact preparation freezes its source fence with the submission.
use super::{Error, StartRequest};
use crate::{
    agent::{
        model::AgentModelResolver,
        writing_commands::{
            PrepareCommandError, PrepareRequest, PrepareStatus, PreviewError, RestoreRequest,
            SubmissionPreview, status_value,
        },
        writing_submission::{
            PreparedWritingCompaction, WritingSubmissionReceipt, WritingSubmissionService,
        },
    },
    storage::AppStore,
};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Prepare {
    pub request: PrepareRequest,
    pub expected_revision: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub preview: SubmissionPreview,
    pub expected_revision: u64,
}
impl Preview {
    fn from_prepared(value: PreparedWritingCompaction, manuscript: uuid::Uuid) -> Self {
        Self {
            preview: SubmissionPreview::from_prepared(value.submission, manuscript),
            expected_revision: value.expected_revision,
        }
    }
}

pub(crate) async fn prepare(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    input: Prepare,
) -> Result<Preview, PrepareCommandError> {
    let request = input.request;
    let manuscript = request.context.manuscript_id;
    let record = service
        .prepare_preview(
            models,
            request.clone().into(),
            Some(input.expected_revision),
        )
        .await
        .map_err(|cause| request.preparation_failure(cause))?;
    Ok(Preview {
        preview: SubmissionPreview::from_record(record, manuscript),
        expected_revision: input.expected_revision,
    })
}

pub(crate) async fn restore(
    service: &WritingSubmissionService,
    models: &AgentModelResolver,
    request: RestoreRequest,
) -> Result<Preview, PreviewError> {
    let value = service
        .restore_compaction(
            models,
            WritingSubmissionReceipt {
                conversation_id: request.conversation_id,
                request_id: request.request_id,
                manuscript_id: request.manuscript_id,
            },
        )
        .await?;
    Ok(Preview::from_prepared(value, request.manuscript_id))
}

#[tauri::command]
pub(crate) async fn agent_prepare_writing_compaction(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
    expected_revision: u64,
) -> Result<Preview, PrepareCommandError> {
    prepare(
        &service,
        &models,
        Prepare {
            request,
            expected_revision,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrepareStatusResponse {
    schema_version: u8,
    intent: Prepare,
    status: PrepareStatus<Preview>,
}
impl PrepareStatusResponse {
    pub(crate) fn from_status(
        input: Prepare,
        value: crate::data_interface::WritingPrepareStatus,
    ) -> Self {
        let status = status_value(&input.request, value, |record| Preview {
            preview: SubmissionPreview::from_record(record, input.request.context.manuscript_id),
            expected_revision: input.expected_revision,
        });
        Self {
            schema_version: 1,
            intent: input,
            status,
        }
    }
}
pub(crate) async fn prepare_status(
    service: &WritingSubmissionService,
    input: Prepare,
) -> Result<PrepareStatusResponse, PrepareCommandError> {
    let value = service
        .read_prepare_status(
            input
                .request
                .preparation_intent(Some(input.expected_revision)),
        )
        .await
        .map_err(|cause| input.request.storage_failure(cause))?;
    Ok(PrepareStatusResponse::from_status(input, value))
}
#[tauri::command]
pub(crate) async fn agent_read_writing_compaction_prepare_status(
    service: State<'_, WritingSubmissionService>,
    request: PrepareRequest,
    expected_revision: u64,
) -> Result<PrepareStatusResponse, PrepareCommandError> {
    prepare_status(
        &service,
        Prepare {
            request,
            expected_revision,
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn agent_restore_writing_compaction(
    service: State<'_, WritingSubmissionService>,
    models: State<'_, AgentModelResolver>,
    request: RestoreRequest,
) -> Result<Preview, PreviewError> {
    restore(&service, &models, request).await
}

pub(super) async fn validate_frozen_fence(
    store: &AppStore,
    request: &StartRequest,
) -> Result<(), Error> {
    let store = store.clone();
    let conversation = request.conversation_id;
    let operation = request.request_id;
    let expected = request.expected_revision;
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(|_| Error::HostUnavailable)?;
        let saved = crate::agent::writing_submission::read_compaction_revision_in(
            &db,
            conversation,
            operation,
        )
        .map_err(|_| Error::HostUnavailable)?;
        if saved.is_some_and(|saved| saved != expected) {
            return Err(Error::ConfirmationMismatch);
        }
        Ok(())
    })
    .await
    .map_err(|_| Error::HostUnavailable)?
}
