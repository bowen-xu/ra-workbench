//! Shared wire receipts and errors for same-turn text admission.
use research_agent_core::steering::SteeringError;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code")]
pub enum SteerError {
    Invalid,
    Conflict,
    ExpectedTurnMismatch {
        #[serde(rename = "actualTurnId")]
        actual_turn_id: Uuid,
    },
    NotSteerable {
        #[serde(rename = "taskKind")]
        task_kind: research_agent_core::protocol::TaskKind,
    },
    Closed,
    Capacity,
    Unavailable,
}
impl From<SteeringError> for SteerError {
    fn from(error: SteeringError) -> Self {
        match error {
            SteeringError::Invalid => Self::Invalid,
            SteeringError::Conflict => Self::Conflict,
            SteeringError::ExpectedTurnMismatch { actual_turn_id } => {
                Self::ExpectedTurnMismatch { actual_turn_id }
            }
            SteeringError::NotSteerable { task_kind } => Self::NotSteerable { task_kind },
            SteeringError::Closed => Self::Closed,
            SteeringError::Capacity => Self::Capacity,
            SteeringError::Unavailable => Self::Unavailable,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteerResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub input_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredInput {
    pub input_id: Uuid,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_steerable_task_has_a_structured_wire_error() {
        let error = SteerError::from(SteeringError::NotSteerable {
            task_kind: research_agent_core::protocol::TaskKind::Compact,
        });
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "code": "notSteerable", "taskKind": "compact"
            })
        );
    }
}
