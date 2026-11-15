use super::*;

#[test]
fn closed_owner_keeps_proof_readable_without_authorizing_a_replay_or_missing_receipt() {
    let f = Fixture::new();
    let original = f.prepare();
    f.db.shutdown().unwrap();
    assert!(
        matches!(f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),QueuePreviewPrepareStatus::Committed{state,..} if state.record==original.record&&state.queue==original.queue)
    );
    assert!(f.db.replay_queue_preview_prepare(&f.intent).is_err());
    assert!(f.db.prepare_queue_preview(&f.intent, f.source()).is_err());
    let mut missing = f.intent.clone();
    missing.request_id = Uuid::new_v4();
    assert!(
        matches!(f.db.read_queue_preview_prepare_status(&missing).unwrap(),QueuePreviewPrepareStatus::Unconfirmed(detail) if detail.safe_cause=="owner_closed_receipt_unconfirmed")
    );
}

#[test]
fn status_does_not_project_an_accepted_suffix_or_change_frozen_queue_result() {
    use crate::data_interface::journal::JournalCommand;
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, TaskKind},
    };
    use research_data_runtime::WriteOptions;
    let f = Fixture::new();
    let original = f.prepare();
    let event = AgentEvent::new(
        ConversationId::from_uuid(f.intent.conversation_id),
        1,
        0,
        f.intent.request_id,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: original.record.turn_id,
            context_snapshot_id: original.record.context_snapshot_id,
            model: original.record.input.model.clone(),
        },
    )
    .unwrap();
    f.db.write(
        JournalCommand::new(
            ConversationId::from_uuid(f.intent.conversation_id),
            Uuid::new_v4(),
            0,
            vec![event],
            None,
            1_790_000_000_000,
        )
        .unwrap(),
        WriteOptions::default(),
    )
    .unwrap();
    let observer = f.observer();
    let head = || {
        observer
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap()
    };
    assert_eq!(head(), 0);
    assert!(
        matches!(f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),QueuePreviewPrepareStatus::Committed{state,..} if state.record==original.record&&state.queue==original.queue)
    );
    assert_eq!(head(), 0);
    assert_eq!(
        observer
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [f.intent.conversation_id.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(f.counts(&observer), (1, 1, 1));
}

#[test]
fn fresh_preflight_rejects_stale_revision_paused_and_ineligible_input_without_writes() {
    for variant in 0..3 {
        let f = Fixture::new();
        let mut intent = f.intent.clone();
        match variant {
            0 => intent.expected_queue_revision = 0,
            1 => f.edit(QueueEdit::Pause),
            _ => intent.input_id = Uuid::new_v4(),
        }
        let before = f.queue();
        assert!(f.db.replay_queue_preview_prepare(&intent).is_err());
        assert!(f.db.prepare_queue_preview(&intent, f.source()).is_err());
        assert_eq!(f.queue(), before);
        assert_eq!(f.counts(&f.observer()), (0, 0, 0));
    }
}
