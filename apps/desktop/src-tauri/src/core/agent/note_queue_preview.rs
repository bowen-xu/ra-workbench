//! Queue heads freeze through the same note preparation transaction as ordinary input.
use super::{
    model::AgentModelResolver,
    note_queue::{ReadRequest, Response as QueueResponse, Scope},
    note_queue_admission::authorize_record,
    note_submission::{
        self, SelectedRequest,
        source::{Selection, SourceReceipt},
    },
    queue_contract::{self, CommandError},
    reading_note_submission,
    repository::input_queue,
};
use crate::storage::AppStore;
use research_agent_core::{input_queue::InputQueue, submission::SubmissionRecord};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Submission {
    Note(note_submission::PrepareRequest),
    ReadingNote(reading_note_submission::PrepareRequest),
}
impl Submission {
    fn selected(self) -> SelectedRequest {
        match self {
            Self::Note(r) => SelectedRequest {
                user_input: r.user_input,
                conversation_id: r.conversation_id,
                request_id: r.request_id,
                text: r.text,
                context: Selection::Page(r.context),
                retrieval: r.retrieval,
                model: r.model,
                session_config: None,
                turn_config: None,
            },
            Self::ReadingNote(r) => SelectedRequest {
                user_input: r.user_input,
                conversation_id: r.conversation_id,
                request_id: r.request_id,
                text: r.text,
                context: Selection::Reading(r.context),
                retrieval: r.retrieval,
                model: r.model,
                session_config: None,
                turn_config: None,
            },
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    pub input_id: Uuid,
    pub expected_revision: u64,
    pub submission: Submission,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelRequest {
    pub conversation_id: Uuid,
    pub scope: Scope,
    pub request_id: Uuid,
    pub expected_revision: u64,
    pub input_id: Uuid,
    pub preview_request_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub schema_version: u8,
    pub input_id: Uuid,
    pub scope: Scope,
    pub record: FrozenRecord,
    pub queue: QueueResponse,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenRecord {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub input: research_agent_core::submission::SubmissionInput,
}
impl From<SubmissionRecord> for FrozenRecord {
    fn from(record: SubmissionRecord) -> Self {
        Self {
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            input: record.input,
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(tag = "code", content = "reason", rename_all = "camelCase")]
pub enum Error {
    Invalid,
    Preparation(note_submission::Error),
    Queue(CommandError),
}
impl From<note_submission::Error> for Error {
    fn from(error: note_submission::Error) -> Self {
        Self::Preparation(error)
    }
}
impl From<input_queue::Error> for Error {
    fn from(error: input_queue::Error) -> Self {
        Self::Queue(error.into())
    }
}
impl From<CommandError> for Error {
    fn from(error: CommandError) -> Self {
        Self::Queue(error)
    }
}
fn response(record: SubmissionRecord, queue: InputQueue, scope: Scope) -> Result<Preview, Error> {
    note_submission::validate_record(&record, scope.resource())?;
    Ok(Preview {
        schema_version: 1,
        input_id: queue
            .preparing()
            .ok_or(Error::Queue(CommandError::Corrupt))?,
        scope,
        record: FrozenRecord::from(record),
        queue: QueueResponse {
            scope,
            queue: queue_contract::state(queue)?,
        },
    })
}

#[tauri::command]
pub(crate) async fn agent_prepare_note_queue_preview(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    prepare(&store, &models, request).await
}
pub(crate) async fn prepare(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    if request.input_id.is_nil() || request.expected_revision >= 9_007_199_254_740_991 {
        return Err(Error::Invalid);
    }
    let selected = request.submission.selected();
    let scope = Scope::from(selected.context.resource());
    let (record, queue) = note_submission::prepare_selected_with(
        store,
        models,
        selected,
        move |tx, receipt, input| {
            input_queue::prepare_preview_in(
                tx,
                receipt.conversation_id,
                receipt.request_id,
                request.expected_revision,
                request.input_id,
                input,
            )
            .map_err(Error::from)
        },
    )
    .await?;
    response(record, queue, scope)
}

#[tauri::command]
pub(crate) async fn agent_read_pending_note_queue_preview(
    store: State<'_, AppStore>,
    request: ReadRequest,
) -> Result<Option<Preview>, Error> {
    read_pending(&store, request).await
}
pub(crate) async fn read_pending(
    store: &AppStore,
    request: ReadRequest,
) -> Result<Option<Preview>, Error> {
    request.scope.validate(request.conversation_id)?;
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|_| Error::Queue(CommandError::Unavailable))?;
        let pending =
            input_queue::read_pending_preview_checked(&mut db, request.conversation_id, |db| {
                request.scope.authorize(db, request.conversation_id)
            })?;
        pending
            .map(|(record, queue)| response(record, queue, request.scope))
            .transpose()
    })
    .await
    .map_err(|_| Error::Queue(CommandError::Unavailable))?
}

#[tauri::command]
pub(crate) async fn agent_cancel_note_queue_preview(
    store: State<'_, AppStore>,
    request: CancelRequest,
) -> Result<QueueResponse, Error> {
    cancel(&store, request).await
}
pub(crate) async fn cancel(
    store: &AppStore,
    request: CancelRequest,
) -> Result<QueueResponse, Error> {
    request.scope.validate(request.conversation_id)?;
    if request.expected_revision >= 9_007_199_254_740_991 {
        return Err(Error::Invalid);
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|_| Error::Queue(CommandError::Unavailable))?;
        let queue = input_queue::cancel_preview_checked(
            &mut db,
            request.conversation_id,
            request.request_id,
            request.expected_revision,
            request.input_id,
            request.preview_request_id,
            |db| {
                authorize_record(
                    db,
                    SourceReceipt {
                        conversation_id: request.conversation_id,
                        request_id: request.preview_request_id,
                        resource: request.scope.resource(),
                    },
                )
                .map(|_| ())
            },
        )?;
        Ok(QueueResponse {
            scope: request.scope,
            queue: queue_contract::state(queue)?,
        })
    })
    .await
    .map_err(|_| Error::Queue(CommandError::Unavailable))?
}
