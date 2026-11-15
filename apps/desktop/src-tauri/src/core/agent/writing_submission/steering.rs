use super::*;
use crate::agent::repository::steering::SqliteSteeringStore;
use research_agent_core::steering::{SteeringError, SteeringInput, SteeringScope};
use rusqlite::OptionalExtension;

impl WritingSubmissionService {
    /// Recheck host binding and the original frozen manuscript scope in the same
    /// transaction as queue admission, including idempotent retries.
    pub async fn steer(&self, manuscript: Uuid, input: SteeringInput) -> Result<(), SteeringError> {
        let scope = input.scope;
        let store = self.store.clone();
        SqliteSteeringStore::new(self.store.shared_agent_connection())
            .enqueue_with_resources(
                input,
                move |db| check_scope(db, manuscript, scope),
                move |db, input| crate::agent::skills::steering::prepare(db, &store, input),
            )
            .await
    }

    pub async fn recover_steering(
        &self,
        manuscript: Uuid,
        scope: SteeringScope,
    ) -> Result<Vec<SteeringInput>, SteeringError> {
        SqliteSteeringStore::new(self.store.shared_agent_connection())
            .recoverable_checked(scope, move |db| check_scope(db, manuscript, scope))
            .await
    }
}

fn check_scope(
    db: &rusqlite::Connection,
    manuscript: Uuid,
    scope: SteeringScope,
) -> Result<(), SteeringError> {
    check_binding(db, scope.conversation_id, manuscript).map_err(|error| match error {
        WritingSubmissionError::Unbound => SteeringError::Closed,
        _ => SteeringError::Unavailable,
    })?;
    let request: Option<String> = db
        .query_row(
            "select request_id from agent_submissions where conversation_id=?1 and turn_id=?2",
            params![scope.conversation_id.to_string(), scope.turn_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| SteeringError::Unavailable)?;
    let request = request.ok_or(SteeringError::Closed)?;
    let request = Uuid::parse_str(&request).map_err(|_| SteeringError::Invalid)?;
    let record = read(db, scope.conversation_id, request)
        .map_err(|_| SteeringError::Unavailable)?
        .ok_or(SteeringError::Closed)?;
    validate_scope(&record, manuscript).map_err(|_| SteeringError::Closed)?;
    Ok(())
}

#[cfg(test)]
#[path = "steering/tests.rs"]
mod tests;
