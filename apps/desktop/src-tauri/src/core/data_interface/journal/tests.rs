use super::*;
use research_agent_core::{conversation::HistoryQuery, protocol::ItemKind};
use sha2::{Digest, Sha256};
#[path = "coalescer_tests.rs"]
mod coalescing;
#[path = "coalescer_recovery_tests.rs"]
mod coalescing_recovery;
#[path = "coalescer_rollback_tests.rs"]
mod coalescing_rollback;
#[cfg(target_os = "macos")]
#[path = "hook_prompt_tests.rs"]
mod hook_prompt;

const ACCEPTED_AT: u64 = 1_790_000_000_123;

fn fixture() -> (SqlProjector, Uuid) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let epoch = Uuid::new_v4();
    db.execute(
        "insert into data_runtime_state values(1,1,?1,0,'ready')",
        [epoch.to_string()],
    )
    .unwrap();
    (SqlProjector::new(Arc::new(Mutex::new(db))), epoch)
}
fn event(id: ConversationId, seq: u64, payload: EventPayload) -> AgentEvent {
    AgentEvent::new(id, seq, 0, Uuid::from_u128(42), payload).unwrap()
}
fn command(
    id: ConversationId,
    expected: u64,
    events: Vec<AgentEvent>,
    title: Option<&str>,
) -> JournalCommand {
    JournalCommand::new(
        id,
        Uuid::new_v4(),
        expected,
        events,
        title.map(str::to_owned),
        ACCEPTED_AT,
    )
    .unwrap()
}
fn accepted(epoch: Uuid, position: u64, command: JournalCommand) -> Accepted<JournalCommand, u64> {
    Accepted {
        epoch,
        position,
        operation_id: command.operation(),
        fingerprint: Sha256::digest(serde_json::to_vec(&command).unwrap()).into(),
        result: command.result_count().unwrap(),
        command,
    }
}

#[test]
fn folds_across_commands_and_flushes_before_completed_text_slices() {
    let (projector, epoch) = fixture();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let first = event(
        id,
        1,
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: item,
            kind: ItemKind::AssistantMessage,
        },
    );
    let delta1 = event(
        id,
        2,
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: item,
            text: "你\0".into(),
        },
    );
    let delta2 = event(
        id,
        3,
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: item,
            text: "🙂e\u{301}".into(),
        },
    );
    let complete = event(
        id,
        4,
        EventPayload::ItemCompleted {
            turn_id: turn,
            item_id: item,
        },
    );
    let records = [
        accepted(
            epoch,
            1,
            command(id, 0, vec![first.clone(), delta1.clone()], Some("stream")),
        ),
        accepted(epoch, 2, command(id, 2, vec![delta2.clone()], None)),
        accepted(epoch, 3, command(id, 3, vec![complete.clone()], None)),
    ];
    projector.raw.lock().unwrap().execute_batch("create table body_updates(value integer); insert into body_updates values(0); create trigger count_body_updates after update of body on agent_messages begin update body_updates set value=value+1; end;").unwrap();
    projector.apply_batch(&records).unwrap();
    let mut db = projector.raw.lock().unwrap();
    let body: String = db
        .query_row("select body from agent_messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(body, "你\0🙂e\u{301}");
    assert_eq!(
        db.query_row("select value from body_updates", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(db.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='storedTextSlice'", [], |r|r.get::<_,u64>(0)).unwrap(), 2);
    let history =
        storage::read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 10).unwrap())
            .unwrap();
    assert_eq!(history.items, vec![first, delta1, delta2, complete]);
    let times: (String, String) = db
        .query_row(
            "select created_at,updated_at from agent_conversations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let expected: String = db
        .query_row(
            "select strftime('%Y-%m-%dT%H:%M:%fZ',?1/1000.0,'unixepoch')",
            [ACCEPTED_AT],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(times, (expected.clone(), expected));
    drop(db);
    projector.apply_batch(&records).unwrap();
    assert_eq!(projector.state().unwrap().materialized_position, 3);
}

#[test]
fn empty_creation_and_pending_cas_share_one_admission_sequence() {
    let (projector, epoch) = fixture();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let creation = command(id, 0, vec![], Some("created"));
    assert!(matches!(
        projector.validate(&creation, &[]),
        Ok(Admission::New(0))
    ));
    let pending = [accepted(epoch, 1, creation)];
    let start = event(
        id,
        1,
        EventPayload::ItemStarted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
        },
    );
    let next = command(id, 0, vec![start], None);
    assert!(matches!(
        projector.validate(&next, &pending),
        Ok(Admission::New(1))
    ));
    let mut stale = next.clone();
    stale.expected_count = 1;
    stale.events[0] = event(id, 2, stale.events[0].payload().clone());
    assert!(matches!(
        projector.validate(&stale, &pending),
        Err(JournalError::RevisionConflict)
    ));
    projector.apply_batch(&pending).unwrap();
    assert_eq!(
        projector
            .lookup(pending[0].operation_id)
            .unwrap()
            .unwrap()
            .result,
        0
    );
}

#[test]
fn deleted_unbound_creation_is_rejected_before_admission_and_keeps_its_original_proof() {
    let (projector, epoch) = fixture();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let original = command(id, 0, vec![], Some("original"));
    let operation = original.operation();
    projector
        .apply_batch(&[accepted(epoch, 1, original)])
        .unwrap();
    {
        let db = projector.raw.lock().unwrap();
        db.execute("insert into agent_conversation_deletions(request_id,conversation_id,request_hash) values(?1,?2,?3)",params![Uuid::new_v4().to_string(),id.as_uuid().to_string(),"a".repeat(64)]).unwrap();
        db.execute(
            "delete from agent_conversations where id=?1",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    }
    let proof = projector.lookup(operation).unwrap().unwrap();
    assert_eq!(proof.result, 0);
    assert!(matches!(
        projector.validate(&command(id, 0, vec![], Some("retry")), &[]),
        Err(JournalError::NotFound)
    ));
    let again = projector.lookup(operation).unwrap().unwrap();
    assert_eq!(
        (
            again.operation_id,
            again.position,
            again.fingerprint,
            again.result
        ),
        (
            proof.operation_id,
            proof.position,
            proof.fingerprint,
            proof.result
        )
    );
}

#[test]
fn body_receipt_control_and_watermark_rollback_together() {
    let (projector, epoch) = fixture();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let records = [accepted(
        epoch,
        1,
        command(
            id,
            0,
            vec![
                event(
                    id,
                    1,
                    EventPayload::ItemStarted {
                        turn_id: turn,
                        item_id: item,
                        kind: ItemKind::AssistantMessage,
                    },
                ),
                event(
                    id,
                    2,
                    EventPayload::TextDelta {
                        turn_id: turn,
                        item_id: item,
                        text: "rolled back".into(),
                    },
                ),
            ],
            Some("atomic"),
        ),
    )];
    projector.raw.lock().unwrap().execute_batch("create trigger reject_watermark before update of materialized_position on data_runtime_state begin select raise(abort,'synthetic projection failure'); end;").unwrap();
    assert!(projector.apply_batch(&records).is_err());
    let db = projector.raw.lock().unwrap();
    for table in [
        "agent_conversations",
        "agent_journal_records",
        "agent_messages",
        "data_operation_receipts",
    ] {
        assert_eq!(
            db.query_row(&format!("select count(*) from {table}"), [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    assert!(
        !db.query_row(
            "select coalesce_text from data_projection_control",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    drop(db);
    assert_eq!(projector.state().unwrap().materialized_position, 0);
    projector
        .raw
        .lock()
        .unwrap()
        .execute_batch("drop trigger reject_watermark")
        .unwrap();
    projector.apply_batch(&records).unwrap();
    assert_eq!(projector.state().unwrap().materialized_position, 1);
}

#[test]
fn legacy_batch_hash_replay_survives_archive_but_new_content_is_rejected() {
    let (projector, _) = fixture();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let mut db = projector.raw.lock().unwrap();
    storage::create_conversation(&mut db, id, "legacy").unwrap();
    let value = event(
        id,
        1,
        EventPayload::ItemStarted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
        },
    );
    let mut replay = command(id, 0, vec![value], None);
    storage::append_batch(&mut db, id, replay.batch_id, 0, &replay.events).unwrap();
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    drop(db);
    assert!(matches!(
        projector.validate(&replay, &[]),
        Ok(Admission::ExistingLegacy(1))
    ));
    replay.events[0] = event(
        id,
        1,
        EventPayload::ItemCompleted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
        },
    );
    assert!(matches!(
        projector.validate(&replay, &[]),
        Err(JournalError::BatchConflict)
    ));
    replay.batch_id = Uuid::new_v4();
    assert!(matches!(
        projector.validate(&replay, &[]),
        Err(JournalError::Archived)
    ));
}

#[test]
fn unsupported_command_and_wrong_epoch_do_not_materialize() {
    let (projector, epoch) = fixture();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let mut value = command(id, 0, vec![], Some("schema"));
    value.schema_version = 2;
    assert!(matches!(
        projector.validate(&value, &[]),
        Err(JournalError::InvalidInput)
    ));
    value.schema_version = 1;
    let mut record = accepted(epoch, 1, value);
    record.epoch = Uuid::new_v4();
    assert_eq!(
        projector.apply_batch(&[record]).unwrap_err(),
        JournalError::Corrupt
    );
    assert_eq!(projector.state().unwrap().materialized_position, 0);
}
