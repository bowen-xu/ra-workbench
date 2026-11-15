//! Saved-note access to the conversation queue. Edits do not prepare or run a turn.
use super::{
    note_submission,
    queue_contract::{self, CommandError, Edit, QueueState},
    repository::input_queue::{self, Error},
};
use crate::data_interface::{QueueEditIntent, QueueEditScope};
use crate::storage::AppStore;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

pub use super::note_scope::Scope;
impl Scope {
    pub(super) fn validate(self, conversation: Uuid) -> Result<(), CommandError> {
        if conversation.is_nil() || self.resource().id().is_nil() {
            Err(CommandError::Invalid)
        } else {
            Ok(())
        }
    }
    pub(super) fn authorize(self, db: &Connection, conversation: Uuid) -> Result<(), Error> {
        self.resource()
            .check_binding(db, conversation)
            .map_err(|error| match error {
                note_submission::Error::Unbound => Error::Closed,
                _ => Error::Storage,
            })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRequest {
    pub conversation_id: Uuid,
    pub scope: Scope,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest {
    pub conversation_id: Uuid,
    pub scope: Scope,
    pub request_id: Uuid,
    pub expected_revision: u64,
    #[serde(default)]
    pub expected_epoch: Option<Uuid>,
    pub edit: Edit,
}
impl EditRequest {
    pub(crate) fn into_intent(self) -> Result<QueueEditIntent, CommandError> {
        self.scope.validate(self.conversation_id)?;
        if self.request_id.is_nil()
            || self.expected_epoch.is_some_and(|epoch| epoch.is_nil())
            || self.expected_revision >= 9_007_199_254_740_991
        {
            return Err(CommandError::Invalid);
        }
        let scope = match self.scope {
            Scope::Note { resource_id } => QueueEditScope::Note { resource_id },
            Scope::ReadingNote { resource_id } => QueueEditScope::ReadingNote { resource_id },
        };
        Ok(QueueEditIntent {
            conversation_id: self.conversation_id,
            request_id: self.request_id,
            expected_revision: self.expected_revision,
            expected_epoch: self.expected_epoch,
            scope,
            edit: self.edit.try_into()?,
        })
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub(super) scope: Scope,
    #[serde(flatten)]
    pub(super) queue: QueueState,
}

#[tauri::command]
pub(crate) async fn agent_read_note_input_queue(
    store: State<'_, AppStore>,
    request: ReadRequest,
) -> Result<Response, CommandError> {
    read(&store, request).await
}
#[tauri::command]
pub(crate) async fn agent_edit_note_input_queue(
    store: State<'_, AppStore>,
    request: EditRequest,
) -> Result<Response, CommandError> {
    edit(&store, request).await
}

pub(crate) async fn read(store: &AppStore, request: ReadRequest) -> Result<Response, CommandError> {
    request.scope.validate(request.conversation_id)?;
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| CommandError::Unavailable)?;
        let queue = input_queue::read_checked(&mut db, request.conversation_id, |db| {
            request.scope.authorize(db, request.conversation_id)
        })?;
        Ok(Response {
            scope: request.scope,
            queue: queue_contract::state(queue)?,
        })
    })
    .await
    .map_err(|_| CommandError::Unavailable)?
}

pub(crate) async fn edit(store: &AppStore, request: EditRequest) -> Result<Response, CommandError> {
    let scope = request.scope;
    let queue = queue_contract::execute_edit(
        store,
        request.into_intent()?,
        super::repository::WorkerRetention::default(),
    )
    .await?;
    Ok(Response {
        scope,
        queue: queue_contract::state(queue)?,
    })
}

#[tauri::command]
pub(crate) async fn agent_read_note_input_queue_edit_status(
    store: State<'_, AppStore>,
    request: EditRequest,
) -> Result<queue_contract::EditStatusResponse, CommandError> {
    edit_status(&store, request).await
}
pub(crate) async fn edit_status(
    store: &AppStore,
    request: EditRequest,
) -> Result<queue_contract::EditStatusResponse, CommandError> {
    queue_contract::read_edit_status(
        store,
        request.into_intent()?,
        super::repository::WorkerRetention::default(),
    )
    .await
}

#[cfg(test)]
#[path = "note_queue/tests.rs"]
mod tests;
