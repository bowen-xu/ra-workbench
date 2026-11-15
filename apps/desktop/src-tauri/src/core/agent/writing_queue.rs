//! User queue edits only. Preparation/admission remain native execution operations.
pub use super::queue_contract::{CommandError, Edit};
use super::writing_submission::WritingSubmissionService;
use crate::data_interface::{QueueEditIntent, QueueEditScope};
use research_agent_core::input_queue::InputQueue;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRequest {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub request_id: Uuid,
    pub expected_revision: u64,
    #[serde(default)]
    pub expected_epoch: Option<Uuid>,
    pub edit: Edit,
}
impl EditRequest {
    pub(crate) fn into_intent(self) -> Result<QueueEditIntent, CommandError> {
        if self.conversation_id.is_nil()
            || self.manuscript_id.is_nil()
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
            scope: QueueEditScope::Manuscript {
                manuscript_id: self.manuscript_id,
            },
            edit: self.edit.try_into()?,
        })
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueResponse {
    manuscript_id: Uuid,
    #[serde(flatten)]
    queue: super::queue_contract::QueueState,
}
pub(crate) fn response(queue: InputQueue, manuscript: Uuid) -> Result<QueueResponse, CommandError> {
    Ok(QueueResponse {
        manuscript_id: manuscript,
        queue: super::queue_contract::state(queue)?,
    })
}
#[tauri::command]
pub async fn agent_read_writing_input_queue(
    service: State<'_, WritingSubmissionService>,
    request: ReadRequest,
) -> Result<QueueResponse, CommandError> {
    let queue = service
        .read_input_queue(request.manuscript_id, request.conversation_id)
        .await?;
    response(queue, request.manuscript_id)
}
#[tauri::command]
pub async fn agent_edit_writing_input_queue(
    service: State<'_, WritingSubmissionService>,
    request: EditRequest,
) -> Result<QueueResponse, CommandError> {
    edit(&service, request).await
}
pub(crate) async fn edit(
    service: &WritingSubmissionService,
    request: EditRequest,
) -> Result<QueueResponse, CommandError> {
    let manuscript = request.manuscript_id;
    let intent = request.into_intent()?;
    let queue = service
        .edit_input_queue_intent(intent.clone())
        .await
        .map_err(|cause| super::queue_contract::failure(&intent, cause))?;
    response(queue, manuscript)
}

#[tauri::command]
pub async fn agent_read_writing_input_queue_edit_status(
    service: State<'_, WritingSubmissionService>,
    request: EditRequest,
) -> Result<super::queue_contract::EditStatusResponse, CommandError> {
    edit_status(&service, request).await
}
pub(crate) async fn edit_status(
    service: &WritingSubmissionService,
    request: EditRequest,
) -> Result<super::queue_contract::EditStatusResponse, CommandError> {
    let intent = request.into_intent()?;
    let status = service
        .read_input_queue_edit_status(intent.clone())
        .await
        .map_err(|cause| super::queue_contract::failure(&intent, cause))?;
    super::queue_contract::edit_status(intent, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::input_queue::{QueueEdit, QueuedInput};
    #[test]
    fn wire_edits_exclude_execution_and_unknown_fields() {
        assert!(serde_json::from_value::<Edit>(serde_json::json!({"type":"add","inputId":Uuid::new_v4(),"text":"test","origin":{"kind":"note","resourceId":Uuid::new_v4()}})).is_err());
        for kind in ["prepare", "admitted", "release"] {
            assert!(
                serde_json::from_value::<Edit>(
                    serde_json::json!({"type":kind,"inputId":Uuid::new_v4()})
                )
                .is_err()
            );
        }
        assert!(
            serde_json::from_value::<Edit>(serde_json::json!({"type":"pause","model":"override"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<Edit>(
                serde_json::json!({"type":"add","inputId":Uuid::new_v4(),"text":"test"})
            )
            .is_ok()
        );
    }
    #[test]
    fn response_keeps_full_scope_and_preparation_identity() {
        let scope = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let input = Uuid::new_v4();
        let queue = InputQueue::new(scope)
            .unwrap()
            .apply(
                0,
                QueueEdit::Add(QueuedInput {
                    user_input: vec![],
                    origin: None,
                    id: input,
                    text: "test".into(),
                }),
            )
            .unwrap()
            .apply(1, QueueEdit::Prepare(input))
            .unwrap();
        let value = serde_json::to_value(response(queue, manuscript).unwrap()).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["revision"], 2);
        assert_eq!(value["conversationId"], scope.to_string());
        assert_eq!(value["manuscriptId"], manuscript.to_string());
        assert_eq!(value["preparingInputId"], input.to_string());
    }
}
