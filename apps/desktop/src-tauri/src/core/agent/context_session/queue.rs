//! Conversation-scoped adapter for the shared durable input queue. No second scheduler.
use super::{Error, Preview, check_conversation};
use crate::agent::{
    queue_contract::{self, CommandError, Confirmation, Edit},
    repository::input_queue,
};
use crate::storage::AppStore;
use crate::{
    agent::repository::WorkerRetention,
    data_interface::{QueueEditIntent, QueueEditScope},
};
use research_agent_core::submission::SubmissionRecord;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;
#[path = "queue/admission_managed.rs"]
mod admission_managed;
pub(super) use admission_managed::admit_managed;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub conversation_id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub expected_revision: u64,
    #[serde(default)]
    pub expected_epoch: Option<Uuid>,
    pub edit: Edit,
}
impl EditRequest {
    pub(crate) fn into_intent(self) -> Result<QueueEditIntent, CommandError> {
        if self.conversation_id.is_nil()
            || self.request_id.is_nil()
            || self.expected_epoch.is_some_and(|epoch| epoch.is_nil())
            || self.expected_revision >= 9_007_199_254_740_991
        {
            return Err(CommandError::Invalid);
        }
        Ok(QueueEditIntent {
            conversation_id: self.conversation_id,
            request_id: self.request_id,
            expected_revision: self.expected_revision,
            expected_epoch: self.expected_epoch,
            scope: QueueEditScope::Context,
            edit: self.edit.try_into()?,
        })
    }
}
pub(super) fn authorize(db: &Connection, id: Uuid) -> Result<(), input_queue::Error> {
    check_conversation(db, id).map_err(|error| match error {
        Error::ConversationUnavailable => input_queue::Error::Closed,
        _ => input_queue::Error::Storage,
    })
}
#[tauri::command]
pub(crate) async fn agent_read_context_input_queue(
    store: State<'_, AppStore>,
    request: Scope,
) -> Result<queue_contract::QueueState, CommandError> {
    read_queue(&store, request).await
}

pub(crate) async fn read_queue(
    store: &AppStore,
    request: Scope,
) -> Result<queue_contract::QueueState, CommandError> {
    read_queue_retained(store, request, WorkerRetention::default()).await
}
pub(crate) async fn read_queue_retained(
    store: &AppStore,
    request: Scope,
    retention: WorkerRetention,
) -> Result<queue_contract::QueueState, CommandError> {
    let owned = store.clone();
    retention
        .spawn(move || {
            let mut db = owned.connection().map_err(|_| CommandError::Unavailable)?;
            queue_contract::state(input_queue::read_checked(
                &mut db,
                request.conversation_id,
                |db| authorize(db, request.conversation_id),
            )?)
        })
        .await
        .map_err(|_| CommandError::Unavailable)?
}

#[tauri::command]
pub(crate) async fn agent_edit_context_input_queue(
    store: State<'_, AppStore>,
    request: EditRequest,
) -> Result<queue_contract::QueueState, CommandError> {
    edit_queue(&store, request).await
}

pub(crate) async fn edit_queue(
    store: &AppStore,
    request: EditRequest,
) -> Result<queue_contract::QueueState, CommandError> {
    edit_queue_retained(store, request, WorkerRetention::default()).await
}

pub(crate) async fn edit_queue_retained(
    store: &AppStore,
    request: EditRequest,
    retention: WorkerRetention,
) -> Result<queue_contract::QueueState, CommandError> {
    queue_contract::state(
        queue_contract::execute_edit(store, request.into_intent()?, retention).await?,
    )
}
#[tauri::command]
pub(crate) async fn agent_read_context_input_queue_edit_status(
    store: State<'_, AppStore>,
    request: EditRequest,
) -> Result<queue_contract::EditStatusResponse, CommandError> {
    edit_status(&store, request).await
}
pub(crate) async fn edit_status(
    store: &AppStore,
    request: EditRequest,
) -> Result<queue_contract::EditStatusResponse, CommandError> {
    queue_contract::read_edit_status(store, request.into_intent()?, WorkerRetention::default())
        .await
}

#[tauri::command]
pub(crate) async fn agent_read_pending_context_queue_preview(
    store: State<'_, AppStore>,
    request: Scope,
) -> Result<Option<Preview>, Error> {
    pending_queue(&store, request).await
}

pub(crate) async fn pending_queue(
    store: &AppStore,
    request: Scope,
) -> Result<Option<Preview>, Error> {
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        let pending =
            input_queue::read_pending_preview_checked(&mut db, request.conversation_id, |db| {
                authorize(db, request.conversation_id)
            })
            .map_err(|_| Error::Unavailable)?;
        pending
            .map(|(record, queue)| {
                let mut preview = Preview::from(record);
                preview.queue = Some(Confirmation {
                    input_id: queue.preparing().ok_or(Error::Unavailable)?,
                    expected_revision: queue.revision(),
                });
                Ok(preview)
            })
            .transpose()
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(super) fn restore_preview(
    db: &mut Connection,
    record: SubmissionRecord,
) -> Result<Preview, Error> {
    let id = record.conversation_id;
    let link: Option<(String, Option<u64>)> = db.query_row(
        "select p.input_id,a.expected_revision from agent_input_queue_previews p left join agent_input_queue_admissions a on a.input_id=p.input_id where p.conversation_id=?1 and p.request_id=?2",
        params![id.to_string(),record.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional().map_err(|_| Error::Unavailable)?;
    let mut preview = Preview::from(record);
    if let Some((input, admitted_revision)) = link {
        let input_id = Uuid::parse_str(&input).map_err(|_| Error::Unavailable)?;
        let expected_revision = match admitted_revision {
            Some(revision) => revision,
            None => {
                let state = input_queue::read_checked(db, id, |db| authorize(db, id))
                    .map_err(|_| Error::Unavailable)?;
                if state.preparing() != Some(input_id) {
                    return Err(Error::ConfirmationMismatch);
                }
                state.revision()
            }
        };
        preview.queue = Some(Confirmation {
            input_id,
            expected_revision,
        });
    }
    Ok(preview)
}
#[cfg(test)]
pub(super) fn admit(
    db: &mut Connection,
    record: &SubmissionRecord,
    confirmation: Option<Confirmation>,
    resume: bool,
) -> Result<(), Error> {
    let restored = restore_preview(db, record.clone())?;
    match (restored.queue, confirmation) {
        (None, None) => Ok(()),
        (Some(saved), Some(confirmation)) if saved.input_id == confirmation.input_id => {
            let authorize = |db: &Connection| authorize(db, record.conversation_id);
            let result = if resume {
                input_queue::admit_and_resume_checked(
                    db,
                    record.conversation_id,
                    record.request_id,
                    confirmation.expected_revision,
                    confirmation.input_id,
                    record.input.clone(),
                    authorize,
                )
            } else {
                input_queue::admit_checked(
                    db,
                    record.conversation_id,
                    record.request_id,
                    confirmation.expected_revision,
                    confirmation.input_id,
                    record.input.clone(),
                    authorize,
                )
            };
            result.map_err(admission_error)?;
            Ok(())
        }
        _ => Err(Error::ConfirmationMismatch),
    }
}

fn admission_error(error: input_queue::Error) -> Error {
    match error {
        input_queue::Error::Queue(research_agent_core::input_queue::QueueError::Invalid) => {
            Error::InvalidInput
        }
        input_queue::Error::Queue(_) => Error::ConfirmationMismatch,
        input_queue::Error::Closed => Error::ConversationUnavailable,
        // Storage or corrupt evidence is not proof of a revision or input mismatch.
        input_queue::Error::Storage | input_queue::Error::Corrupt => Error::Unavailable,
    }
}

#[cfg(test)]
#[path = "queue/tests.rs"]
mod tests;
