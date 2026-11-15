//! Text-only steering of an explicitly bound, frozen note turn.
use super::{
    note_scope::Scope,
    note_submission,
    repository::{steering::SqliteSteeringStore, submissions::read},
    steering_contract::{RecoveredInput, SteerError, SteerResponse},
};
use crate::storage::AppStore;
use research_agent_core::steering::{SteeringError, SteeringInput, SteeringScope};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SteerRequest {
    pub conversation_id: Uuid,
    pub scope: Scope,
    pub turn_id: Uuid,
    pub input_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryRequest {
    pub conversation_id: Uuid,
    pub scope: Scope,
    pub turn_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub scope: Scope,
    pub turn_id: Uuid,
    pub inputs: Vec<RecoveredInput>,
}

#[tauri::command]
pub(crate) async fn agent_steer_note_turn(
    store: State<'_, AppStore>,
    request: SteerRequest,
) -> Result<SteerResponse, SteerError> {
    steer(&store, request).await
}
pub(crate) async fn steer(
    store: &AppStore,
    request: SteerRequest,
) -> Result<SteerResponse, SteerError> {
    let turn = SteeringScope {
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
    };
    let resource_store = store.clone();
    SqliteSteeringStore::new(store.shared_agent_connection())
        .enqueue_with_resources(
            SteeringInput {
                scope: turn,
                input_id: request.input_id,
                text: request.text,
                user_input: request.user_input,
                skill_instructions: vec![],
            },
            move |db| check_scope(db, request.scope, turn),
            move |db, input| super::skills::steering::prepare(db, &resource_store, input),
        )
        .await
        .map_err(SteerError::from)?;
    Ok(SteerResponse {
        schema_version: 1,
        conversation_id: turn.conversation_id,
        turn_id: turn.turn_id,
        input_id: request.input_id,
    })
}

#[tauri::command]
pub(crate) async fn agent_read_note_steering_recovery(
    store: State<'_, AppStore>,
    request: RecoveryRequest,
) -> Result<RecoveryResponse, SteerError> {
    recover(&store, request).await
}
pub(crate) async fn recover(
    store: &AppStore,
    request: RecoveryRequest,
) -> Result<RecoveryResponse, SteerError> {
    let turn = SteeringScope {
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
    };
    let inputs = SqliteSteeringStore::new(store.shared_agent_connection())
        .recoverable_checked(turn, move |db| check_scope(db, request.scope, turn))
        .await
        .map_err(SteerError::from)?;
    Ok(RecoveryResponse {
        schema_version: 1,
        conversation_id: turn.conversation_id,
        scope: request.scope,
        turn_id: turn.turn_id,
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

/// Runs inside the same write transaction as admission/recovery, including retries.
/// Current note content is deliberately not read: steering cannot change the frozen source.
fn check_scope(db: &Connection, host: Scope, turn: SteeringScope) -> Result<(), SteeringError> {
    if turn.conversation_id.is_nil() || turn.turn_id.is_nil() || host.resource().id().is_nil() {
        return Err(SteeringError::Invalid);
    }
    host.resource()
        .check_binding(db, turn.conversation_id)
        .map_err(|error| match error {
            note_submission::Error::Unbound => SteeringError::Closed,
            _ => SteeringError::Unavailable,
        })?;
    let request: Option<String> = db
        .query_row(
            "select request_id from agent_submissions where conversation_id=?1 and turn_id=?2",
            params![turn.conversation_id.to_string(), turn.turn_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| SteeringError::Unavailable)?;
    let request = Uuid::parse_str(&request.ok_or(SteeringError::Closed)?)
        .map_err(|_| SteeringError::Invalid)?;
    let record = read(db, turn.conversation_id, request)
        .map_err(|_| SteeringError::Unavailable)?
        .ok_or(SteeringError::Closed)?;
    note_submission::validate_record(&record, host.resource()).map_err(|_| SteeringError::Closed)
}
