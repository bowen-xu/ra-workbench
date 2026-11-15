use super::tests::{chain, marker, write};
use super::*;
use crate::agent::repository::input_queue;
use research_agent_core::{
    input_queue::{QueueEdit, QueuedInput},
    submission::SubmissionInput,
};

fn add(db: &mut Connection, conversation: Uuid, revision: u64, text: &str) -> Uuid {
    let id = Uuid::new_v4();
    input_queue::apply_checked(
        db,
        conversation,
        Uuid::new_v4(),
        revision,
        QueueEdit::Add(QueuedInput {
            id,
            text: text.into(),
            user_input: vec![],
            origin: None,
        }),
        |_| Ok(()),
    )
    .unwrap();
    id
}
fn queue(db: &mut Connection, conversation: Uuid) -> research_agent_core::input_queue::InputQueue {
    input_queue::read_checked(db, conversation, |_| Ok(())).unwrap()
}

#[test]
fn rollback_retains_frozen_claim_and_rejects_its_old_confirmation_revision() {
    let mut db = Connection::open_in_memory().unwrap();
    let (first, second) = chain(&mut db);
    let id = first.conversation_id;
    let head = add(&mut db, id, 0, "queued evidence");
    let tail = add(&mut db, id, 1, "keep order");
    input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        2,
        QueueEdit::PauseInput(tail),
        |_| Ok(()),
    )
    .unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "queued evidence".into(),
        model: first.binding.model.clone(),
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    let request = Uuid::new_v4();
    let (frozen, before) =
        input_queue::prepare_preview_checked(&mut db, id, request, 3, head, input.clone(), |_| {
            Ok(())
        })
        .unwrap();
    write(&mut db, &marker(&second, 11), Uuid::new_v4()).unwrap();
    let after = queue(&mut db, id);
    assert!(after.paused());
    assert_eq!(after.revision(), before.revision() + 1);
    assert_eq!(after.inputs(), before.inputs());
    assert_eq!(after.preparing(), Some(head));
    assert!(after.input_paused(tail));
    assert!(
        input_queue::admit_checked(
            &mut db,
            id,
            request,
            before.revision(),
            head,
            input,
            |_| Ok(())
        )
        .is_err()
    );
    let (recovered, current) = input_queue::read_pending_preview_checked(&mut db, id, |_| Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(recovered, frozen);
    assert_eq!(current.revision(), after.revision());
    assert!(current.paused());
    assert_eq!(read_projection(&db, id).unwrap().seq(), 11);
}

#[test]
fn repeated_rollbacks_advance_paused_revision_but_exact_replays_do_not_repause() {
    let mut db = Connection::open_in_memory().unwrap();
    let (first, second) = chain(&mut db);
    let event = marker(&second, 11);
    let batch = Uuid::new_v4();
    write(&mut db, &event, batch).unwrap();
    assert_eq!(queue(&mut db, first.conversation_id).revision(), 1);
    write(&mut db, &marker(&first, 12), Uuid::new_v4()).unwrap();
    let before = queue(&mut db, first.conversation_id);
    assert!(before.paused());
    assert_eq!(before.revision(), 2);
    input_queue::apply_checked(
        &mut db,
        first.conversation_id,
        Uuid::new_v4(),
        2,
        QueueEdit::Resume,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(write(&mut db, &event, batch).unwrap(), 11);
    let after = queue(&mut db, first.conversation_id);
    assert_eq!(after.revision(), 3);
    assert!(!after.paused());
    assert!(after.inputs().is_empty());
}

#[test]
fn queue_or_marker_failure_rolls_back_queue_window_and_history() {
    for queue_failure in [true, false] {
        let mut db = Connection::open_in_memory().unwrap();
        let (first, second) = chain(&mut db);
        add(&mut db, first.conversation_id, 0, "preserved input");
        let before = queue(&mut db, first.conversation_id);
        db.execute_batch(if queue_failure {
            "create trigger fault before update on agent_input_queues begin select raise(abort,'synthetic queue failure'); end;"
        } else {
            "create trigger fault before insert on agent_items when new.ordinal=10 begin select raise(abort,'synthetic marker failure'); end;"
        }).unwrap();
        assert!(write(&mut db, &marker(&second, 11), Uuid::new_v4()).is_err());
        let after = queue(&mut db, first.conversation_id);
        assert_eq!(after.revision(), before.revision());
        assert_eq!(after.inputs(), before.inputs());
        assert!(!after.paused());
        assert_eq!(
            read_active(&db, first.conversation_id)
                .unwrap()
                .unwrap()
                .window_id,
            second.window_id
        );
        assert_eq!(
            read_projection(&db, first.conversation_id).unwrap().seq(),
            10
        );
    }
}

#[test]
fn corrupt_queue_cannot_publish_a_rollback_or_change_active_window() {
    let mut db = Connection::open_in_memory().unwrap();
    let (first, second) = chain(&mut db);
    add(&mut db, first.conversation_id, 0, "preserved input");
    db.execute(
        "update agent_input_queues set snapshot_json='{}' where conversation_id=?1",
        [first.conversation_id.to_string()],
    )
    .unwrap();
    assert!(write(&mut db, &marker(&second, 11), Uuid::new_v4()).is_err());
    assert_eq!(
        read_active(&db, first.conversation_id)
            .unwrap()
            .unwrap()
            .window_id,
        second.window_id
    );
    assert_eq!(
        read_projection(&db, first.conversation_id).unwrap().seq(),
        10
    );
}

#[test]
fn rollback_fences_unstarted_admission_after_reopen_until_explicit_resume() {
    use research_agent_core::protocol::{EventPayload, TaskKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback-queue.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let (first, second) = chain(&mut db);
    let id = first.conversation_id;
    let input_id = add(&mut db, id, 0, "confirmed but not started");
    input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        1,
        QueueEdit::Prepare(input_id),
        |_| Ok(()),
    )
    .unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "confirmed but not started".into(),
        model: first.binding.model.clone(),
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    let request = Uuid::new_v4();
    let (record, receipt_queue) =
        input_queue::admit_checked(&mut db, id, request, 2, input_id, input.clone(), |_| Ok(()))
            .unwrap();
    write(&mut db, &marker(&second, 11), Uuid::new_v4()).unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    // Reading the old receipt is harmless and must not resume the current queue.
    let (retry, old_snapshot) =
        input_queue::admit_checked(&mut db, id, request, 2, input_id, input, |_| Ok(())).unwrap();
    assert_eq!(retry, record);
    assert_eq!(old_snapshot, receipt_queue);
    assert!(queue(&mut db, id).paused());
    assert_eq!(
        input_queue::list_unstarted_checked(&mut db, id, None, 64, |_| Ok(()))
            .unwrap()
            .len(),
        1
    );
    let event = AgentEvent::new(
        ConversationId::from_uuid(id),
        12,
        1,
        request,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
        },
    )
    .unwrap();
    let batch = Uuid::new_v4();
    assert!(write(&mut db, &event, batch).is_err());
    assert_eq!(read_projection(&db, id).unwrap().seq(), 11);
    let paused = queue(&mut db, id);
    input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        paused.revision(),
        QueueEdit::Resume,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(write(&mut db, &event, batch).unwrap(), 12);
    assert!(
        input_queue::list_unstarted_checked(&mut db, id, None, 64, |_| Ok(()))
            .unwrap()
            .is_empty()
    );
    // Later pauses cannot turn an exact started batch retry into a new execution.
    let current = queue(&mut db, id);
    input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        current.revision(),
        QueueEdit::Pause,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(write(&mut db, &event, batch).unwrap(), 12);
    assert_eq!(read_projection(&db, id).unwrap().seq(), 12);
}
