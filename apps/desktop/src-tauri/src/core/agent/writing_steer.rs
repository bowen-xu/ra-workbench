//! Same-turn text admission; no provider/model/context override is accepted.
pub use super::steering_contract::{SteerError, SteerResponse};
use super::writing_submission::WritingSubmissionService;
use research_agent_core::steering::{SteeringInput, SteeringScope};
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SteerRequest {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub turn_id: Uuid,
    pub input_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
}
#[tauri::command]
pub async fn agent_steer_writing_turn(
    service: State<'_, WritingSubmissionService>,
    request: SteerRequest,
) -> Result<SteerResponse, SteerError> {
    steer(&service, request).await
}

pub(crate) async fn steer(
    service: &WritingSubmissionService,
    request: SteerRequest,
) -> Result<SteerResponse, SteerError> {
    service
        .steer(
            request.manuscript_id,
            SteeringInput {
                scope: SteeringScope {
                    conversation_id: request.conversation_id,
                    turn_id: request.turn_id,
                },
                input_id: request.input_id,
                text: request.text,
                user_input: request.user_input,
                skill_instructions: vec![],
            },
        )
        .await
        .map_err(SteerError::from)?;
    Ok(SteerResponse {
        schema_version: 1,
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
        input_id: request.input_id,
    })
}
