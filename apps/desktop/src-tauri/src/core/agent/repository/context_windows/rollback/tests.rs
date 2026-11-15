use super::super::tests::{append, fixture};
use super::*;
use research_agent_core::protocol::{EventPayload, ItemKind, TaskKind, TurnOutcome};

fn finish_user(db: &mut Connection, checkpoint: &ContextCheckpoint, start: u64) {
    let id = Uuid::new_v4();
    for (offset, payload) in [
        EventPayload::ItemStarted {
            turn_id: checkpoint.turn_id,
            item_id: id,
            kind: ItemKind::UserMessage,
        },
        EventPayload::TextDelta {
            turn_id: checkpoint.turn_id,
            item_id: id,
            text: "original evidence".into(),
        },
        EventPayload::ItemCompleted {
            turn_id: checkpoint.turn_id,
            item_id: id,
        },
        EventPayload::TurnFinished {
            turn_id: checkpoint.turn_id,
            outcome: TurnOutcome::Completed,
        },
    ]
    .into_iter()
    .enumerate()
    {
        append(
            db,
            checkpoint.conversation_id,
            start + offset as u64,
            payload,
        );
    }
}
pub(super) fn chain(db: &mut Connection) -> (ContextCheckpoint, ContextCheckpoint) {
    let first = fixture(db);
    install(db, &first).unwrap();
    finish_user(db, &first, 1);
    let mut second = first.clone();
    second.window_id = Uuid::new_v4();
    second.turn_id = Uuid::new_v4();
    second.previous_window_id = Some(first.window_id);
    second.source_watermark = 6;
    append(
        db,
        first.conversation_id,
        5,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: second.turn_id,
            context_snapshot_id: Uuid::new_v4(),
            model: second.binding.model.clone(),
        },
    );
    install(db, &second).unwrap();
    finish_user(db, &second, 6);
    (first, second)
}
pub(super) fn marker(saved: &ContextCheckpoint, seq: u64) -> AgentEvent {
    AgentEvent::new(
        ConversationId::from_uuid(saved.conversation_id),
        seq,
        1,
        Uuid::new_v4(),
        EventPayload::ConversationRolledBack {
            turn_id: saved.turn_id,
            user_turns: 1,
        },
    )
    .unwrap()
}
pub(super) fn write(
    db: &mut Connection,
    event: &AgentEvent,
    batch: Uuid,
) -> Result<u64, JournalError> {
    crate::agent::repository::append_batch(
        db,
        event.conversation_id(),
        batch,
        event.seq() - 1,
        &[event],
    )
}

#[test]
fn rollback_reopens_surviving_window_and_exact_batch_replay_does_not_reactivate_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollback.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let (first, second) = chain(&mut db);
    db.execute_batch(include_str!(
        "../../../../../../migrations/0131_agent_compact_session_start.sql"
    ))
    .unwrap();
    db.execute_batch(include_str!(
        "../../../../../../migrations/0221_agent_compact_session_start_queue.sql"
    ))
    .unwrap();
    session_start::enqueue(&db, &second).unwrap();
    let event = marker(&second, 11);
    let batch = Uuid::new_v4();
    assert_eq!(write(&mut db, &event, batch).unwrap(), 11);
    assert!(
        session_start::take(&mut db, first.conversation_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        read_active(&db, first.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        first.window_id
    );
    let history = read_projection(&db, first.conversation_id).unwrap();
    assert!(history.turn(second.turn_id).is_none());
    assert!(history.turn(first.turn_id).is_some());
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    assert_eq!(
        read_active(&db, first.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        first.window_id
    );
    assert_eq!(
        read_projection(&db, first.conversation_id).unwrap().seq(),
        11
    );
    let last = marker(&first, 12);
    write(&mut db, &last, Uuid::new_v4()).unwrap();
    assert!(read_active(&db, first.conversation_id).unwrap().is_none());
    assert_eq!(write(&mut db, &event, batch).unwrap(), 11);
    assert!(read_active(&db, first.conversation_id).unwrap().is_none());
    drop(db);
    let db = Connection::open(&path).unwrap();
    assert!(read_active(&db, first.conversation_id).unwrap().is_none());
    assert_eq!(
        read_projection(&db, first.conversation_id).unwrap().seq(),
        12
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_context_checkpoints",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("select count(*) from agent_items", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        12
    );
}

#[test]
fn pointer_or_marker_failure_rolls_back_both_history_and_window() {
    for fail_pointer in [true, false] {
        let mut db = Connection::open_in_memory().unwrap();
        let (first, second) = chain(&mut db);
        db.execute_batch(if fail_pointer {
            "create trigger fault before update on agent_active_context_windows begin select raise(abort,'synthetic pointer failure'); end;"
        } else {
            "create trigger fault before insert on agent_items when new.ordinal=10 begin select raise(abort,'synthetic marker failure'); end;"
        }).unwrap();
        assert!(write(&mut db, &marker(&second, 11), Uuid::new_v4()).is_err());
        assert_eq!(
            read_active(&db, first.conversation_id)
                .unwrap()
                .unwrap()
                .window_id,
            second.window_id
        );
        let history = read_projection(&db, first.conversation_id).unwrap();
        assert_eq!(history.seq(), 10);
        assert!(history.turn(second.turn_id).is_some());
    }
}

#[test]
fn stale_marker_and_corrupt_chain_publish_no_rollback() {
    let mut db = Connection::open_in_memory().unwrap();
    let (first, second) = chain(&mut db);
    assert!(write(&mut db, &marker(&first, 11), Uuid::new_v4()).is_err());
    // Simulate a damaged database, bypassing the normal immutable-row guard.
    db.execute_batch("drop trigger agent_context_checkpoints_immutable")
        .unwrap();
    db.execute(
        "update agent_context_checkpoints set checkpoint_json='{}' where id=?1",
        [first.window_id.to_string()],
    )
    .unwrap();
    assert!(write(&mut db, &marker(&second, 11), Uuid::new_v4()).is_err());
    assert_eq!(
        read_projection(&db, first.conversation_id).unwrap().seq(),
        10
    );
    assert_eq!(
        read_active(&db, first.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        second.window_id
    );
}
