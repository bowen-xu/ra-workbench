use super::*;
use crate::agent::repository::{append_batch, create_conversation};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TurnOutcome},
};
fn event(scope: Uuid, outcome: TurnOutcome) -> AgentEvent {
    AgentEvent::new(
        ConversationId::from_uuid(scope),
        1,
        0,
        Uuid::new_v4(),
        EventPayload::TurnFinished {
            turn_id: Uuid::new_v4(),
            outcome,
        },
    )
    .unwrap()
}
fn setup() -> (Connection, Uuid) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 94);
    let scope = Uuid::new_v4();
    create_conversation(&mut db, ConversationId::from_uuid(scope), "terminal").unwrap();
    (db, scope)
}
#[test]
fn failure_outcomes_pause_and_exact_journal_replay_does_not_repause() {
    for outcome in [
        TurnOutcome::Failed,
        TurnOutcome::Canceled,
        TurnOutcome::Interrupted,
    ] {
        let (mut db, scope) = setup();
        let input_id = Uuid::new_v4();
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id: input_id,
                text: "keep".into(),
            }),
            |_| Ok(()),
        )
        .unwrap();
        let event = event(scope, outcome);
        let batch = Uuid::new_v4();
        append_batch(
            &mut db,
            ConversationId::from_uuid(scope),
            batch,
            0,
            std::slice::from_ref(&event),
        )
        .unwrap();
        let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
        assert!(queue.paused());
        assert_eq!(queue.revision(), 2);
        assert_eq!(queue.inputs()[0].id, input_id);
        apply_checked(&mut db, scope, Uuid::new_v4(), 2, QueueEdit::Resume, |_| {
            Ok(())
        })
        .unwrap();
        append_batch(
            &mut db,
            ConversationId::from_uuid(scope),
            batch,
            0,
            &[event],
        )
        .unwrap();
        assert!(!read_checked(&mut db, scope, |_| Ok(())).unwrap().paused());
    }
}
#[test]
fn failed_pause_rolls_back_terminal_and_receipt_together() {
    let (mut db, scope) = setup();
    db.execute_batch("create trigger reject_pause before insert on agent_input_queues begin select raise(abort,'fixture'); end;").unwrap();
    let event = event(scope, TurnOutcome::Interrupted);
    let batch = Uuid::new_v4();
    assert!(
        append_batch(
            &mut db,
            ConversationId::from_uuid(scope),
            batch,
            0,
            std::slice::from_ref(&event)
        )
        .is_err()
    );
    assert_eq!(
        db.query_row("select count(*) from agent_items", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from agent_journal_batches", [], |row| row
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger reject_pause;").unwrap();
    append_batch(
        &mut db,
        ConversationId::from_uuid(scope),
        batch,
        0,
        &[event],
    )
    .unwrap();
    assert!(read_checked(&mut db, scope, |_| Ok(())).unwrap().paused());
}
#[test]
fn completed_outcome_preserves_queue_and_existing_pause_is_idempotent() {
    let (mut db, scope) = setup();
    append_batch(
        &mut db,
        ConversationId::from_uuid(scope),
        Uuid::new_v4(),
        0,
        &[event(scope, TurnOutcome::Completed)],
    )
    .unwrap();
    assert_eq!(
        read_checked(&mut db, scope, |_| Ok(())).unwrap().revision(),
        0
    );
    apply_checked(&mut db, scope, Uuid::new_v4(), 0, QueueEdit::Pause, |_| {
        Ok(())
    })
    .unwrap();
    append_batch(
        &mut db,
        ConversationId::from_uuid(scope),
        Uuid::new_v4(),
        1,
        &[event(scope, TurnOutcome::Failed)],
    )
    .unwrap();
    let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
    assert!(queue.paused());
    assert_eq!(queue.revision(), 1);
}

#[test]
fn malformed_pause_state_prevents_a_partial_terminal_commit() {
    let (mut db, scope) = setup();
    apply_checked(&mut db, scope, Uuid::new_v4(), 0, QueueEdit::Pause, |_| {
        Ok(())
    })
    .unwrap();
    db.execute("update agent_input_queues set snapshot_json=json_remove(snapshot_json,'$.paused') where conversation_id=?1", [scope.to_string()]).unwrap();
    assert!(
        append_batch(
            &mut db,
            ConversationId::from_uuid(scope),
            Uuid::new_v4(),
            0,
            &[event(scope, TurnOutcome::Failed)]
        )
        .is_err()
    );
    assert_eq!(
        db.query_row("select count(*) from agent_items", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
