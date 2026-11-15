use super::*;
#[path = "tests/fork.rs"]
mod fork;
#[path = "tests/handlers.rs"]
mod handlers;
#[path = "tests/spills.rs"]
mod spills;
use research_agent_core::{
    context::{checkpoint::ContextCheckpoint, compaction_strategy::*},
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind},
};

fn append(db: &mut Connection, conversation: Uuid, watermark: u64, payload: EventPayload) {
    let id = ConversationId::from_uuid(conversation);
    let event = AgentEvent::new(id, watermark + 1, 1, Uuid::new_v4(), payload).unwrap();
    crate::agent::repository::append_batch(db, id, Uuid::new_v4(), watermark, &[event]).unwrap();
}

fn fixture(db: &mut Connection) -> ContextCheckpoint {
    crate::storage_migration_tests::migrate_through(db, 144);
    let conversation = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: None,
    };
    crate::agent::repository::create_conversation(
        db,
        ConversationId::from_uuid(conversation),
        "hook context",
    )
    .unwrap();
    append(
        db,
        conversation,
        0,
        EventPayload::TurnStarted {
            turn_id: turn,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: model.clone(),
        },
    );
    ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::new_v4(),
        conversation_id: conversation,
        turn_id: turn,
        source_watermark: 1,
        previous_window_id: None,
        binding: CompactionBinding {
            model,
            strategy_id: "fixture.text".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::TextSummary("summary".into()),
        retained: vec![],
        canonical_context: None,
    }
}

fn setup(path: &std::path::Path) -> (Connection, CompactionHookRequest) {
    let mut db = Connection::open(path).unwrap();
    let window = fixture(&mut db);
    context_windows::install(&mut db, &window).unwrap();
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let request = CompactionHookRequest {
        operation_id: window.window_id,
        conversation_id: window.conversation_id,
        turn_id: window.turn_id,
        phase: window.phase,
        model: window.binding.model,
        stage: CompactionHookStage::SessionStartCompact,
    };
    super::super::record(&db, &request, CompactionHookNotification::Started, 1).unwrap();
    (db, request)
}
fn result() -> CompactionHookResult {
    CompactionHookResult {
        outcome: CompactionHookOutcome::Stop,
        additional_contexts: vec!["  preserved\n正文".into(), "second".into()],
    }
}

#[test]
fn stop_context_reopens_in_order_and_respects_snapshot_and_window_cutoffs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hooks.sqlite");
    let (mut db, request) = setup(&path);
    record_result(&mut db, &request, &result(), 2).unwrap();
    drop(db);
    let db = Connection::open(path).unwrap();
    let saved = read(&db, request.conversation_id, request.operation_id, 0, 1).unwrap();
    assert_eq!(
        saved,
        vec![ContextBatch {
            turn_id: request.turn_id,
            boundary_seq: 1,
            contexts: result().additional_contexts
        }]
    );
    for (after, through) in [(0, 0), (1, 1)] {
        assert!(
            read(
                &db,
                request.conversation_id,
                request.operation_id,
                after,
                through
            )
            .unwrap()
            .is_empty()
        );
    }
    assert!(
        db.execute(
            "update agent_compact_hook_contexts set contexts_json='[]'",
            []
        )
        .is_err()
    );
    assert_eq!(
        db.query_row(
            "select outcome from agent_compaction_hook_audit where notification='completed'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "stop"
    );
}

#[test]
fn queued_ancestor_context_is_owned_by_active_window_and_unrelated_window_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = setup(&root.path().join("queued.sqlite"));
    let first = context_windows::read_active(&db, request.conversation_id)
        .unwrap()
        .unwrap();
    let mut second = first.clone();
    second.window_id = Uuid::new_v4();
    second.previous_window_id = Some(first.window_id);
    context_windows::install(&mut db, &second).unwrap();
    record_result(&mut db, &request, &result(), 2).unwrap();
    assert_eq!(
        read(&db, request.conversation_id, second.window_id, 0, 1).unwrap()[0].contexts,
        result().additional_contexts
    );
    assert!(
        read(&db, request.conversation_id, first.window_id, 0, 1)
            .unwrap()
            .is_empty()
    );

    let mut sibling = first.clone();
    sibling.window_id = Uuid::new_v4();
    db.execute("insert into agent_context_checkpoints(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json) values(?1,?2,?3,?4,null,?5,?6)", rusqlite::params![sibling.window_id.to_string(), sibling.conversation_id.to_string(), sibling.turn_id.to_string(), sibling.source_watermark, sibling.schema_version, serde_json::to_string(&sibling).unwrap()]).unwrap();
    let unrelated = CompactionHookRequest {
        operation_id: sibling.window_id,
        ..request
    };
    super::super::record(&db, &unrelated, CompactionHookNotification::Started, 3).unwrap();
    assert_eq!(
        record_result(&mut db, &unrelated, &result(), 4),
        Err(Error::Conflict)
    );
}

#[test]
fn context_write_failure_rolls_back_completion_and_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = setup(&root.path().join("hooks.sqlite"));
    db.execute_batch("create trigger fail_context before insert on agent_compact_hook_contexts begin select raise(abort,'fixture'); end;").unwrap();
    assert_eq!(
        record_result(&mut db, &request, &result(), 2),
        Err(Error::Storage)
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compaction_hook_audit where notification='completed'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger fail_context").unwrap();
    record_result(&mut db, &request, &result(), 3).unwrap();
    assert!(record_result(&mut db, &request, &result(), 4).is_err());
}

#[test]
fn invalid_context_cannot_complete_the_hook_or_change_its_window() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = setup(&root.path().join("hooks.sqlite"));
    let mut failed = result();
    failed.outcome = CompactionHookOutcome::Failed;
    assert_eq!(
        record_result(&mut db, &request, &failed, 2),
        Err(Error::Invalid)
    );
    let mut huge = result();
    huge.additional_contexts = vec!["x".repeat(MAX_CONTEXT_BYTES + 1)];
    assert_eq!(
        record_result(&mut db, &request, &huge, 2),
        Err(Error::Invalid)
    );
    let mut wrong = request.clone();
    wrong.operation_id = Uuid::new_v4();
    assert_eq!(
        record_result(&mut db, &wrong, &result(), 2),
        Err(Error::Conflict)
    );
    wrong = request.clone();
    wrong.stage = CompactionHookStage::PostCompact;
    assert_eq!(
        record_result(&mut db, &wrong, &result(), 2),
        Err(Error::Invalid)
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compaction_hook_audit",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn rollback_excludes_context_but_preserves_original_evidence() {
    use research_agent_core::protocol::{EventPayload, ItemKind, TurnOutcome};
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = setup(&root.path().join("hooks.sqlite"));
    record_result(&mut db, &request, &result(), 2).unwrap();
    let user = Uuid::new_v4();
    for (index, event) in [
        EventPayload::ItemStarted {
            turn_id: request.turn_id,
            item_id: user,
            kind: ItemKind::UserMessage,
        },
        EventPayload::ItemCompleted {
            turn_id: request.turn_id,
            item_id: user,
        },
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Canceled,
        },
        EventPayload::ConversationRolledBack {
            turn_id: request.turn_id,
            user_turns: 1,
        },
    ]
    .into_iter()
    .enumerate()
    {
        append(&mut db, request.conversation_id, index as u64 + 1, event);
    }
    assert!(
        read(&db, request.conversation_id, request.operation_id, 0, 5)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_compact_hook_contexts",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
