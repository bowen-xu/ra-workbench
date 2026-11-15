//! Read-only projections over the authoritative Agent journal.
//!
//! This adapter deliberately does not open a writer or infer business state. It
//! is shared by compatibility projections and typed hosts that need to reconcile
//! their own records with a durable Agent turn.

use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, ConversationProjection, TurnOutcome},
};
use rusqlite::{Connection, params};
use uuid::Uuid;

pub fn read_turn_outcome(
    connection: &Connection,
    conversation_id: ConversationId,
    turn_id: Uuid,
) -> rusqlite::Result<Option<TurnOutcome>> {
    let has_journal: bool = connection.query_row(
        "select exists(select 1 from sqlite_master where type in ('table','view') and name='agent_items')",
        [],
        |row| row.get(0),
    )?;
    if !has_journal {
        return Ok(None);
    }
    let mut statement = connection.prepare(
        "select payload_json from agent_items where conversation_id=?1 order by ordinal",
    )?;
    let events = statement.query_map(params![conversation_id.as_uuid().to_string()], |row| {
        let payload: String = row.get(0)?;
        serde_json::from_str::<AgentEvent>(&payload).map_err(|_| rusqlite::Error::InvalidQuery)
    })?;
    let mut projection = ConversationProjection::new(conversation_id);
    for event in events {
        projection
            .apply(&event?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
    }
    Ok(projection.turn(turn_id).and_then(|turn| turn.outcome()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::protocol::{EventPayload, ModelIdentity};

    #[test]
    fn reads_outcome_from_the_core_projection() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "create table agent_items(conversation_id text not null, ordinal integer not null, payload_json text not null);",
            )
            .unwrap();
        let conversation = ConversationId::from_uuid(Uuid::new_v4());
        let turn = Uuid::new_v4();
        let request = Uuid::new_v4();
        let model = ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture-v1".into(),
            revision: None,
        };
        let payloads = [
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: turn,
                context_snapshot_id: Uuid::new_v4(),
                model: model.clone(),
            },
            EventPayload::PhaseChanged {
                turn_id: turn,
                phase: research_agent_core::protocol::TurnPhase::RunningModel,
            },
            EventPayload::TurnFinished {
                turn_id: turn,
                outcome: TurnOutcome::Completed,
            },
        ];
        for (index, payload) in payloads.into_iter().enumerate() {
            let event =
                AgentEvent::new(conversation, (index + 1) as u64, 1, request, payload).unwrap();
            connection
                .execute(
                    "insert into agent_items(conversation_id,ordinal,payload_json) values(?1,?2,?3)",
                    params![conversation.as_uuid().to_string(), index as i64, serde_json::to_string(&event).unwrap()],
                )
                .unwrap();
        }
        assert_eq!(
            read_turn_outcome(&connection, conversation, turn).unwrap(),
            Some(TurnOutcome::Completed)
        );
    }
}
