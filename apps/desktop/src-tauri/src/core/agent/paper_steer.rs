//! Same-turn text admission for the paper reading host.
use super::{
    repository::steering::SqliteSteeringStore,
    steering_contract::{SteerError, SteerResponse},
};
use crate::storage::AppStore;
use research_agent_core::steering::{SteeringError, SteeringInput, SteeringScope};
use rusqlite::{Connection, params};
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SteerRequest {
    pub conversation_id: Uuid,
    pub paper_id: String,
    pub turn_id: Uuid,
    pub input_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
}

#[tauri::command]
pub(crate) async fn agent_steer_paper_turn(
    store: State<'_, AppStore>,
    request: SteerRequest,
) -> Result<SteerResponse, SteerError> {
    steer(store.inner(), request).await
}

pub(crate) async fn steer(
    store: &AppStore,
    request: SteerRequest,
) -> Result<SteerResponse, SteerError> {
    if request.paper_id.trim().is_empty()
        || request.paper_id.len() > 256
        || request.paper_id.contains('\0')
    {
        return Err(SteerError::Invalid);
    }
    let scope = SteeringScope {
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
    };
    let paper_id = request.paper_id.clone();
    let resource_store = store.clone();
    SqliteSteeringStore::new(store.shared_agent_connection())
        .enqueue_with_resources(
            SteeringInput {
                scope,
                input_id: request.input_id,
                text: request.text,
                user_input: request.user_input,
                skill_instructions: vec![],
            },
            move |db| check_scope(db, scope, &paper_id),
            move |db, input| super::skills::steering::prepare(db, &resource_store, input),
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

fn check_scope(db: &Connection, scope: SteeringScope, paper_id: &str) -> Result<(), SteeringError> {
    let allowed: bool = db.query_row(
        "select exists(select 1 from agent_submissions s join agent_conversations c on c.id=s.conversation_id join agent_conversation_bindings b on b.conversation_id=c.id where s.turn_id=?1 and s.conversation_id=?2 and c.archived=0 and b.resource_kind='paper' and b.resource_id=?3)",
        params![scope.turn_id.to_string(), scope.conversation_id.to_string(), paper_id], |row| row.get(0),
    ).map_err(|_| SteeringError::Unavailable)?;
    if allowed {
        Ok(())
    } else {
        Err(SteeringError::Closed)
    }
}
