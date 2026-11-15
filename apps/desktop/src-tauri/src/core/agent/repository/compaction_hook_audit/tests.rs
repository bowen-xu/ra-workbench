use super::*;
use uuid::Uuid;
fn request(id: Uuid) -> CompactionHookRequest {
    CompactionHookRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: id,
        turn_id: Uuid::new_v4(),
        phase: CompactionPhase::MidTurn,
        stage: CompactionHookStage::PreCompact,
        model: research_agent_core::protocol::ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "model".into(),
            revision: Some("revision".into()),
        },
    }
}
fn fixture(path: &std::path::Path) -> (Connection, CompactionHookRequest) {
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut db,
        research_agent_core::conversation::ConversationId::from_uuid(id),
        "hook audit",
    )
    .unwrap();
    let request = request(id);
    let event = research_agent_core::protocol::AgentEvent::new(
        research_agent_core::conversation::ConversationId::from_uuid(id),
        1,
        1,
        Uuid::new_v4(),
        research_agent_core::protocol::EventPayload::TurnStarted {
            turn_id: request.turn_id,
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: request.model.clone(),
        },
    )
    .unwrap();
    crate::agent::repository::append_batch(
        &mut db,
        research_agent_core::conversation::ConversationId::from_uuid(id),
        Uuid::new_v4(),
        0,
        &[event],
    )
    .unwrap();
    (db, request)
}
#[test]
fn compaction_hook_audit_reopens_incomplete_start_without_inventing_completion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("audit.sqlite");
    let (db, request) = fixture(&path);
    record(&db, &request, CompactionHookNotification::Started, 1).unwrap();
    drop(db);
    let db = Connection::open(path).unwrap();
    let (notification, outcome): (String, Option<String>) = db
        .query_row(
            "select notification,outcome from agent_compaction_hook_audit",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(notification, "started");
    assert_eq!(outcome, None);
    assert!(record(&db, &request, CompactionHookNotification::Started, 2).is_err());
    assert!(
        db.execute(
            "update agent_compaction_hook_audit set outcome='continue',notification='completed'",
            []
        )
        .is_err()
    );
}
#[test]
fn compaction_hook_audit_requires_matching_start_and_keeps_all_stages_and_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let (db, mut request) = fixture(&root.path().join("audit.sqlite"));
    let stages = [
        CompactionHookStage::PreCompact,
        CompactionHookStage::PostCompact,
        CompactionHookStage::SessionStartCompact,
    ];
    for (i, outcome) in [
        CompactionHookOutcome::Continue,
        CompactionHookOutcome::Stop,
        CompactionHookOutcome::Failed,
        CompactionHookOutcome::Canceled,
    ]
    .into_iter()
    .enumerate()
    {
        request.operation_id = Uuid::new_v4();
        for stage in stages {
            request.stage = stage;
            let done = CompactionHookNotification::Completed(outcome);
            assert!(record(&db, &request, done, 2).is_err());
            record(&db, &request, CompactionHookNotification::Started, i as i64).unwrap();
            let mut altered = request.clone();
            altered.model.revision = None;
            assert!(record(&db, &altered, done, 2).is_err());
            altered = request.clone();
            altered.phase = CompactionPhase::Manual;
            assert!(record(&db, &altered, done, 2).is_err());
            record(&db, &request, done, 2).unwrap();
            assert!(record(&db, &request, done, 3).is_err());
        }
    }
    let count: i64 = db
        .query_row(
            "select count(*) from agent_compaction_hook_audit",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 24);
    db.execute("pragma foreign_keys=on", []).unwrap();
    db.execute(
        "delete from agent_conversations where id=?1",
        [request.conversation_id.to_string()],
    )
    .unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from agent_compaction_hook_audit",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn output_free_audit_cannot_write_for_wrong_or_finished_task() {
    use research_agent_core::protocol::{AgentEvent, EventPayload, TurnOutcome};
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = fixture(&root.path().join("audit.sqlite"));
    let mut wrong = request.clone();
    wrong.turn_id = Uuid::new_v4();
    assert!(record(&db, &wrong, CompactionHookNotification::Started, 1).is_err());
    record(&db, &request, CompactionHookNotification::Started, 1).unwrap();
    let conversation =
        research_agent_core::conversation::ConversationId::from_uuid(request.conversation_id);
    let finished = AgentEvent::new(
        conversation,
        2,
        2,
        Uuid::new_v4(),
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Canceled,
        },
    )
    .unwrap();
    crate::agent::repository::append_batch(&mut db, conversation, Uuid::new_v4(), 1, &[finished])
        .unwrap();
    assert!(
        record(
            &db,
            &request,
            CompactionHookNotification::Completed(CompactionHookOutcome::Canceled),
            3,
        )
        .is_err()
    );
    let mut next = request.clone();
    next.operation_id = Uuid::new_v4();
    assert!(record(&db, &next, CompactionHookNotification::Started, 3).is_err());
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
fn archived_conversation_cannot_start_or_complete_output_free_hook() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = fixture(&root.path().join("audit.sqlite"));
    record(&db, &request, CompactionHookNotification::Started, 1).unwrap();
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    let mut another = request.clone();
    another.operation_id = Uuid::new_v4();
    assert!(record(&db, &another, CompactionHookNotification::Started, 2).is_err());
    assert!(
        context::record_result(
            &mut db,
            &request,
            &research_agent_core::context::compaction_hooks::CompactionHookResult::without_context(
                CompactionHookOutcome::Continue,
            ),
            2,
        )
        .is_err()
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
