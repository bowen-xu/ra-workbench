//! Terminal input recovery is observational: no queue deletion or new turn admission.
use super::*;
use research_agent_core::protocol::{AgentEvent, EventPayload};

impl SqliteSteeringStore {
    pub(crate) fn recoverable_checked(
        &self,
        scope: SteeringScope,
        check: impl FnOnce(&Connection) -> Result<(), SteeringError> + Send + 'static,
    ) -> SteeringFuture<'_, Vec<SteeringInput>> {
        Box::pin(async move {
            self.run(move |db| {
                check(db)?;
                require_terminal(db, scope)?;
                let mut statement = db.prepare("select input_id,text from agent_steering_inputs where turn_id=?1 order by ordinal").map_err(db_error)?;
                let rows = statement.query_map([scope.turn_id.to_string()], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).map_err(db_error)?;
                let mut inputs = Vec::new();
                for row in rows {
                    let (id, text) = row.map_err(db_error)?;
                    let input = resources::input(db, scope, &id, text)?;
                    input.validate()?;
                    // Partial/conflicting history needs reconciliation, never automatic replay.
                    if !consumption::is_consumed(db, &input)? { inputs.push(input); }
                }
                Ok(inputs)
            }).await
        })
    }
}

impl SteeringRecoveryStore for SqliteSteeringStore {
    fn recoverable(&self, scope: SteeringScope) -> SteeringFuture<'_, Vec<SteeringInput>> {
        self.recoverable_checked(scope, |_| Ok(()))
    }
}

pub(crate) fn require_terminal(db: &Connection, scope: SteeringScope) -> Result<(), SteeringError> {
    let exists: bool = db.query_row(
        "select exists(select 1 from agent_submissions s join agent_conversations c on c.id=s.conversation_id where s.turn_id=?1 and s.conversation_id=?2 and c.archived=0)",
        params![scope.turn_id.to_string(),scope.conversation_id.to_string()], |r| r.get(0),
    ).map_err(db_error)?;
    if !exists {
        return Err(SteeringError::Closed);
    }
    let json: Option<String> = db.query_row(
        "select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 order by ordinal desc limit 1",
        params![scope.conversation_id.to_string(),scope.turn_id.to_string()], |r| r.get(0),
    ).optional().map_err(db_error)?;
    let event: AgentEvent = serde_json::from_str(&json.ok_or(SteeringError::Closed)?)
        .map_err(|_| SteeringError::Invalid)?;
    if *event.conversation_id().as_uuid() != scope.conversation_id {
        return Err(SteeringError::Invalid);
    }
    if !matches!(event.payload(), EventPayload::TurnFinished { turn_id, .. } if *turn_id == scope.turn_id)
    {
        return Err(SteeringError::Closed);
    }
    Ok(())
}
