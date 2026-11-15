use super::*;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use rusqlite::Connection;

#[test]
fn history_requires_complete_matching_events_and_reports_storage_failure() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "create table agent_items(conversation_id text, ordinal integer, payload_json text)",
    )
    .unwrap();
    let input = SteeringInput {
        scope: SteeringScope {
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
        },
        input_id: Uuid::new_v4(),
        text: "frozen input".into(),
        user_input: vec![],
        skill_instructions: vec![],
    };
    let state = |db: &Connection| serde_json::to_value(history(db, &input).unwrap()).unwrap();
    assert_eq!(state(&db), "unconfirmed");
    let append = |ordinal, payload| {
        let event = AgentEvent::new(
            ConversationId::from_uuid(input.scope.conversation_id),
            ordinal,
            0,
            Uuid::new_v4(),
            payload,
        )
        .unwrap();
        db.execute(
            "insert into agent_items values(?1,?2,?3)",
            params![
                input.scope.conversation_id.to_string(),
                ordinal,
                serde_json::to_string(&event).unwrap()
            ],
        )
        .unwrap();
    };
    append(
        1,
        EventPayload::ItemStarted {
            turn_id: input.scope.turn_id,
            item_id: input.input_id,
            kind: ItemKind::UserMessage,
        },
    );
    assert_eq!(state(&db), "incompleteOrConflicting");
    append(
        2,
        EventPayload::TextDelta {
            turn_id: input.scope.turn_id,
            item_id: input.input_id,
            text: input.text.clone(),
        },
    );
    append(
        3,
        EventPayload::ItemCompleted {
            turn_id: input.scope.turn_id,
            item_id: input.input_id,
        },
    );
    assert_eq!(state(&db), "confirmed");
    let mut changed = input.clone();
    changed.text = "different".into();
    assert_eq!(
        serde_json::to_value(history(&db, &changed).unwrap()).unwrap(),
        "incompleteOrConflicting"
    );
    changed = input.clone();
    changed.scope.turn_id = Uuid::new_v4();
    assert_eq!(
        serde_json::to_value(history(&db, &changed).unwrap()).unwrap(),
        "incompleteOrConflicting"
    );
    db.execute_batch("drop table agent_items").unwrap();
    assert!(history(&db, &input).is_err());
}
