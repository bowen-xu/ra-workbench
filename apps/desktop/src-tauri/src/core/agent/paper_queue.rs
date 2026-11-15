//! Durable Tab queue for paper-reading conversations.
//! The queue stores only user text; paper context is frozen when the queued
//! input is actually submitted, so a hidden reader cannot leak stale sources.
use super::{
    queue_contract::{self, CommandError, Edit},
    reading_submission,
    repository::input_queue,
};
use crate::data_interface::{QueueEditIntent, QueueEditScope};
use research_agent_core::input_queue::InputQueue;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRequest {
    pub conversation_id: Uuid,
    pub paper_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest {
    pub conversation_id: Uuid,
    pub paper_id: String,
    pub request_id: Uuid,
    pub expected_revision: u64,
    #[serde(default)]
    pub expected_epoch: Option<Uuid>,
    pub edit: Edit,
}
impl EditRequest {
    pub(crate) fn into_intent(self) -> Result<QueueEditIntent, CommandError> {
        validate(&ReadRequest {
            conversation_id: self.conversation_id,
            paper_id: self.paper_id.clone(),
        })?;
        if self.request_id.is_nil()
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
            scope: QueueEditScope::Paper {
                paper_id: self.paper_id,
            },
            edit: self.edit.try_into()?,
        })
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    paper_id: String,
    #[serde(flatten)]
    queue: queue_contract::QueueState,
}

fn validate(request: &ReadRequest) -> Result<(), CommandError> {
    if request.conversation_id.is_nil()
        || request.paper_id.trim().is_empty()
        || request.paper_id.len() > 256
        || request.paper_id.contains('\0')
    {
        Err(CommandError::Invalid)
    } else {
        Ok(())
    }
}
fn response(queue: InputQueue, paper_id: String) -> Result<Response, CommandError> {
    Ok(Response {
        paper_id,
        queue: queue_contract::state(queue)?,
    })
}
fn authorize(
    db: &Connection,
    conversation: Uuid,
    paper_id: &str,
) -> Result<(), input_queue::Error> {
    reading_submission::check_binding(db, conversation, paper_id).map_err(|error| match error {
        reading_submission::Error::Unbound => input_queue::Error::Closed,
        _ => input_queue::Error::Storage,
    })
}

#[tauri::command]
pub(crate) async fn agent_read_paper_input_queue(
    store: State<'_, crate::storage::AppStore>,
    request: ReadRequest,
) -> Result<Response, CommandError> {
    read(&store, request).await
}
pub(crate) async fn read(
    store: &crate::storage::AppStore,
    request: ReadRequest,
) -> Result<Response, CommandError> {
    validate(&request)?;
    let owned = store.clone();
    let paper = request.paper_id.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| CommandError::Unavailable)?;
        let queue = input_queue::read_checked(&mut db, request.conversation_id, |db| {
            authorize(db, request.conversation_id, &paper)
        })?;
        response(queue, paper)
    })
    .await
    .map_err(|_| CommandError::Unavailable)?
}

#[tauri::command]
pub(crate) async fn agent_edit_paper_input_queue(
    store: State<'_, crate::storage::AppStore>,
    request: EditRequest,
) -> Result<Response, CommandError> {
    edit(&store, request).await
}
pub(crate) async fn edit(
    store: &crate::storage::AppStore,
    request: EditRequest,
) -> Result<Response, CommandError> {
    let paper = request.paper_id.clone();
    let queue = queue_contract::execute_edit(
        store,
        request.into_intent()?,
        super::repository::WorkerRetention::default(),
    )
    .await?;
    response(queue, paper)
}

#[tauri::command]
pub(crate) async fn agent_read_paper_input_queue_edit_status(
    store: State<'_, crate::storage::AppStore>,
    request: EditRequest,
) -> Result<queue_contract::EditStatusResponse, CommandError> {
    edit_status(&store, request).await
}
pub(crate) async fn edit_status(
    store: &crate::storage::AppStore,
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
mod tests {
    use super::*;
    #[test]
    fn wire_rejects_unscoped_queue_fields() {
        let value = serde_json::json!({"conversationId":Uuid::new_v4(),"paperId":"paper","requestId":Uuid::new_v4(),"expectedRevision":0,"edit":{"type":"add","inputId":Uuid::new_v4(),"text":"继续"}});
        assert!(serde_json::from_value::<EditRequest>(value.clone()).is_ok());
        let mut invalid = value;
        invalid["edit"]["origin"] = serde_json::json!({"kind":"manuscript"});
        assert!(serde_json::from_value::<EditRequest>(invalid).is_err());
    }

    #[test]
    fn paper_edit_retains_a_non_nil_expected_epoch() {
        let epoch = Uuid::new_v4();
        let value = serde_json::json!({"conversationId":Uuid::new_v4(),"paperId":"paper",
            "requestId":Uuid::new_v4(),"expectedRevision":0,"expectedEpoch":epoch,
            "edit":{"type":"remove","inputId":Uuid::new_v4()}});
        let request: EditRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.into_intent().unwrap().expected_epoch, Some(epoch));
        let mut invalid = value;
        invalid["expectedEpoch"] = serde_json::json!(Uuid::nil());
        let request: EditRequest = serde_json::from_value(invalid).unwrap();
        assert!(matches!(request.into_intent(), Err(CommandError::Invalid)));
    }
}
