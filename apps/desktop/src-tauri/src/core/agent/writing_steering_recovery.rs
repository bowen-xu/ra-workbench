//! Read-only terminal input recovery; never starts a new model request.
use super::{
    steering_contract::{RecoveredInput, SteerError},
    writing_submission::WritingSubmissionService,
};
use research_agent_core::steering::SteeringScope;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryRequest {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub turn_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub turn_id: Uuid,
    pub inputs: Vec<RecoveredInput>,
}
#[tauri::command]
pub async fn agent_read_writing_steering_recovery(
    service: State<'_, WritingSubmissionService>,
    request: RecoveryRequest,
) -> Result<RecoveryResponse, SteerError> {
    read(&service, request).await
}

pub(crate) async fn read(
    service: &WritingSubmissionService,
    request: RecoveryRequest,
) -> Result<RecoveryResponse, SteerError> {
    let inputs = service
        .recover_steering(
            request.manuscript_id,
            SteeringScope {
                conversation_id: request.conversation_id,
                turn_id: request.turn_id,
            },
        )
        .await
        .map_err(SteerError::from)?;
    Ok(RecoveryResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        manuscript_id: request.manuscript_id,
        turn_id: request.turn_id,
        inputs: inputs
            .into_iter()
            .map(|input| RecoveredInput {
                input_id: input.input_id,
                text: input.text,
                user_input: input.user_input,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_request_cannot_override_input_or_execution() {
        let base = serde_json::json!({"conversationId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),"turnId":Uuid::new_v4()});
        assert!(serde_json::from_value::<RecoveryRequest>(base.clone()).is_ok());
        for key in ["text", "inputId", "model", "resubmit"] {
            let mut value = base.clone();
            value[key] = serde_json::json!("unexpected");
            assert!(serde_json::from_value::<RecoveryRequest>(value).is_err());
        }
    }
}
