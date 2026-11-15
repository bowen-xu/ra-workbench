//! Persist stop semantics without allowing a stale turn to pause another turn.
use super::*;
use research_agent_core::protocol::{AgentEvent, EventPayload};

pub fn pause_for_turn_checked(
    db: &mut Connection,
    conversation: Uuid,
    turn: Uuid,
) -> Result<bool, Error> {
    if conversation.is_nil() || turn.is_nil() {
        return Err(QueueError::Invalid.into());
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    require_open(&tx, conversation)?;
    let json: Option<String> = tx.query_row(
        "select payload_json from agent_items where conversation_id=?1
         and json_extract(payload_json,'$.payload.type') in ('turnStarted','turnFinished') order by ordinal desc limit 1",
        [conversation.to_string()], |row| row.get(0)).optional().map_err(|_| Error::Storage)?;
    let Some(json) = json else {
        return Ok(false);
    };
    let event: AgentEvent = serde_json::from_str(&json).map_err(|_| Error::Corrupt)?;
    if *event.conversation_id().as_uuid() != conversation {
        return Err(Error::Corrupt);
    }
    if !matches!(event.payload(), EventPayload::TurnStarted { turn_id, .. } if *turn_id == turn) {
        return Ok(false);
    }
    let queue = load(&tx, conversation)?;
    if queue.paused() {
        return Ok(true);
    }
    let queue = queue.apply(queue.revision(), QueueEdit::Pause)?;
    let snapshot = Snapshot::encode(&queue)?;
    tx.execute("insert into agent_input_queues(conversation_id,revision,snapshot_json) values(?1,?2,?3)
        on conflict(conversation_id) do update set revision=excluded.revision,snapshot_json=excluded.snapshot_json",
        params![conversation.to_string(),queue.revision(),snapshot]).map_err(|_|Error::Storage)?;
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::repository::create_conversation;
    use research_agent_core::{conversation::ConversationId, protocol::ModelIdentity};
    #[test]
    fn stop_retains_text_pauses_empty_queues_and_ignores_stale_turns() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 94);
        let scope = Uuid::new_v4();
        let turn = Uuid::new_v4();
        create_conversation(&mut db, ConversationId::from_uuid(scope), "stop").unwrap();
        let event = AgentEvent::new(
            ConversationId::from_uuid(scope),
            1,
            0,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: turn,
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
        )
        .unwrap();
        db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)", params![scope.to_string(),serde_json::to_string(&event).unwrap()]).unwrap();
        assert!(!pause_for_turn_checked(&mut db, scope, Uuid::new_v4()).unwrap());
        assert!(pause_for_turn_checked(&mut db, scope, turn).unwrap());
        let first = read_checked(&mut db, scope, |_| Ok(())).unwrap();
        assert!(first.paused());
        assert!(first.inputs().is_empty());
        let id = Uuid::new_v4();
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            first.revision(),
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id,
                text: "keep".into(),
            }),
            |_| Ok(()),
        )
        .unwrap();
        assert!(pause_for_turn_checked(&mut db, scope, turn).unwrap());
        let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
        assert_eq!(queue.revision(), first.revision() + 1);
        assert_eq!(queue.inputs()[0].text, "keep");
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            queue.revision(),
            QueueEdit::Resume,
            |_| Ok(()),
        )
        .unwrap();
        let next = Uuid::new_v4();
        let event = AgentEvent::new(
            ConversationId::from_uuid(scope),
            2,
            0,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: next,
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
        )
        .unwrap();
        db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,1,1,?2)", params![scope.to_string(),serde_json::to_string(&event).unwrap()]).unwrap();
        assert!(!pause_for_turn_checked(&mut db, scope, turn).unwrap());
        assert!(!read_checked(&mut db, scope, |_| Ok(())).unwrap().paused());
        assert!(pause_for_turn_checked(&mut db, scope, next).unwrap());
    }
}
