use super::*;
use research_agent_core::protocol::{AgentEvent, EventPayload, ItemKind};

pub(crate) fn is_consumed(db: &Connection, input: &SteeringInput) -> Result<bool, SteeringError> {
    let mut statement = db.prepare("select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.itemId')=?2 order by ordinal").map_err(db_error)?;
    let rows = statement
        .query_map(
            params![
                input.scope.conversation_id.to_string(),
                input.input_id.to_string()
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(db_error)?;
    let mut started = false;
    let mut complete = false;
    let mut text = String::new();
    for row in rows {
        let event: AgentEvent =
            serde_json::from_str(&row.map_err(db_error)?).map_err(|_| SteeringError::Invalid)?;
        if *event.conversation_id().as_uuid() != input.scope.conversation_id
            || event.payload().turn_id() != input.scope.turn_id
            || complete
        {
            return Err(SteeringError::Invalid);
        }
        match event.payload() {
            EventPayload::ItemStarted {
                kind: ItemKind::UserMessage,
                ..
            } if !started => started = true,
            EventPayload::TextDelta { text: delta, .. } if started => {
                if text.len() + delta.len() > input.text.len() {
                    return Err(SteeringError::Invalid);
                }
                text.push_str(delta);
            }
            EventPayload::ItemCompleted { .. } if started && text == input.text => complete = true,
            _ => return Err(SteeringError::Invalid),
        }
    }
    if started && !complete {
        return Err(SteeringError::Invalid);
    }
    Ok(complete)
}
