use super::*;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind, TurnOutcome},
};

#[path = "tests/fork.rs"]
mod fork;
#[path = "tests/managed_failure.rs"]
mod managed_failure;

fn append(db: &mut Connection, conversation: Uuid, watermark: u64, payload: EventPayload) {
    let id = ConversationId::from_uuid(conversation);
    let event = AgentEvent::new(id, watermark + 1, 1, Uuid::new_v4(), payload).unwrap();
    super::super::append_batch(db, id, Uuid::new_v4(), watermark, &[event]).unwrap();
}

fn setup() -> (Connection, FragmentBatch) {
    setup_on(Connection::open_in_memory().unwrap())
}

fn setup_on(mut db: Connection) -> (Connection, FragmentBatch) {
    crate::storage_migration_tests::migrate_through(&mut db, 144);
    let conversation_id = Uuid::new_v4();
    let turn_id = Uuid::new_v4();
    super::super::create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation_id),
        "fixture",
    )
    .unwrap();
    append(
        &mut db,
        conversation_id,
        0,
        EventPayload::TurnStarted {
            turn_id,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "model".into(),
                revision: None,
            },
        },
    );
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let window =
        context_windows::identity::read_or_create(&mut db, conversation_id, Uuid::now_v7())
            .unwrap()
            .current;
    let user = Uuid::new_v4();
    let anchor = Uuid::new_v4();
    for (offset, payload) in [
        EventPayload::ItemStarted {
            turn_id,
            item_id: user,
            kind: ItemKind::UserMessage,
        },
        EventPayload::TextDelta {
            turn_id,
            item_id: user,
            text: "original research input".into(),
        },
        EventPayload::ItemCompleted {
            turn_id,
            item_id: user,
        },
        EventPayload::ItemStarted {
            turn_id,
            item_id: anchor,
            kind: ItemKind::AssistantMessage,
        },
        EventPayload::TextDelta {
            turn_id,
            item_id: anchor,
            text: "answer".into(),
        },
        EventPayload::ItemCompleted {
            turn_id,
            item_id: anchor,
        },
    ]
    .into_iter()
    .enumerate()
    {
        append(&mut db, conversation_id, 1 + offset as u64, payload);
    }
    (
        db,
        FragmentBatch {
            conversation_id,
            window_id: window,
            boundary_seq: 7,
            turn_id,
            anchor_item_id: anchor,
            fragments: vec!["remaining 0".into(), "save progress".into()],
        },
    )
}

#[test]
fn fragment_records_survive_connection_close_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let (mut db, batch) = setup_on(Connection::open(&path).unwrap());
    record(&mut db, &batch).unwrap();
    drop(db);
    let db = Connection::open(&path).unwrap();
    let restored = read(&db, batch.conversation_id, batch.window_id, 0).unwrap();
    assert!(restored == vec![batch]);
    drop(db);
}

#[test]
fn production_upgrade_records_ordered_fragments_without_mutating_chat_and_replays_exactly() {
    let (mut db, batch) = setup();
    let before: String = db
        .query_row(
            "select group_concat(payload_json) from agent_items",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(record(&mut db, &batch).unwrap(), Recording::Recorded);
    assert_eq!(record(&mut db, &batch).unwrap(), Recording::AlreadyRecorded);
    assert_eq!(
        read(&db, batch.conversation_id, batch.window_id, 0)
            .unwrap()
            .len(),
        1
    );
    let recovered = read(&db, batch.conversation_id, batch.window_id, 0)
        .unwrap()
        .pop()
        .unwrap();
    assert!(recovered == batch);
    assert!(
        read(&db, batch.conversation_id, batch.window_id, 7)
            .unwrap()
            .is_empty()
    );
    assert!(
        read(&db, batch.conversation_id, Uuid::now_v7(), 0)
            .unwrap()
            .is_empty()
    );
    let mut conflict = batch.clone();
    conflict.fragments[0] = "changed".into();
    assert_eq!(record(&mut db, &conflict), Err(Error::Conflict));
    assert!(
        db.execute("update agent_context_fragments set fragments_json='[]'", [])
            .is_err()
    );
    assert_eq!(
        db.query_row(
            "select group_concat(payload_json) from agent_items",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=136",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn failed_insert_preserves_storage_and_retry_does_not_duplicate_fragments() {
    let (mut db, batch) = setup();
    db.execute_batch("create trigger fail_fragment_insert before insert on agent_context_fragments begin select raise(abort, 'fixture write failure'); end;").unwrap();
    assert_eq!(record(&mut db, &batch), Err(Error::Storage));
    assert!(
        read(&db, batch.conversation_id, batch.window_id, 0)
            .unwrap()
            .is_empty()
    );
    db.execute_batch("drop trigger fail_fragment_insert")
        .unwrap();
    assert_eq!(record(&mut db, &batch).unwrap(), Recording::Recorded);
    assert_eq!(record(&mut db, &batch).unwrap(), Recording::AlreadyRecorded);
}

#[test]
fn rolled_back_anchor_is_not_restored_but_original_fragment_evidence_remains() {
    let (mut db, batch) = setup();
    record(&mut db, &batch).unwrap();
    append(
        &mut db,
        batch.conversation_id,
        7,
        EventPayload::TurnFinished {
            turn_id: batch.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    assert_eq!(
        read(&db, batch.conversation_id, batch.window_id, 0)
            .unwrap()
            .len(),
        1
    );
    append(
        &mut db,
        batch.conversation_id,
        8,
        EventPayload::ConversationRolledBack {
            turn_id: batch.turn_id,
            user_turns: 1,
        },
    );
    assert!(
        read(&db, batch.conversation_id, batch.window_id, 0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.query_row("select count(*) from agent_context_fragments", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        1
    );
}

#[test]
fn stale_journal_or_wrong_window_cannot_record_a_new_batch() {
    let (mut db, batch) = setup();
    let mut invalid = batch.clone();
    invalid.window_id = Uuid::now_v7();
    assert_eq!(record(&mut db, &invalid), Err(Error::Conflict));
    invalid = batch.clone();
    invalid.boundary_seq -= 1;
    assert_eq!(record(&mut db, &invalid), Err(Error::Conflict));
    invalid = batch.clone();
    invalid.fragments.push("extra".into());
    assert_eq!(record(&mut db, &invalid), Err(Error::Invalid));
    assert!(
        read(&db, batch.conversation_id, batch.window_id, 0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fragments_wait_for_the_entire_tool_batch() {
    let (mut db, mut batch) = setup();
    let calls = [Uuid::new_v4(), Uuid::new_v4()];
    let mut seq = 7;
    for call in calls {
        let item = Uuid::new_v4();
        for payload in [
            EventPayload::ItemStarted {
                turn_id: batch.turn_id,
                item_id: item,
                kind: ItemKind::ToolCall {
                    call_id: call,
                    name: "read".into(),
                    provider_call_id: None,
                    namespace: None,
                },
            },
            EventPayload::TextDelta {
                turn_id: batch.turn_id,
                item_id: item,
                text: "{}".into(),
            },
            EventPayload::ItemCompleted {
                turn_id: batch.turn_id,
                item_id: item,
            },
        ] {
            append(&mut db, batch.conversation_id, seq, payload);
            seq += 1;
        }
    }
    for (index, call) in calls.into_iter().enumerate() {
        let item = Uuid::new_v4();
        for payload in [
            EventPayload::ItemStarted {
                turn_id: batch.turn_id,
                item_id: item,
                kind: ItemKind::ToolResult {
                    call_id: call,
                    is_error: Some(false),
                },
            },
            EventPayload::TextDelta {
                turn_id: batch.turn_id,
                item_id: item,
                text: "result".into(),
            },
            EventPayload::ItemCompleted {
                turn_id: batch.turn_id,
                item_id: item,
            },
        ] {
            append(&mut db, batch.conversation_id, seq, payload);
            seq += 1;
        }
        batch.boundary_seq = seq;
        batch.anchor_item_id = item;
        if index == 0 {
            assert_eq!(record(&mut db, &batch), Err(Error::Invalid));
        } else {
            assert_eq!(record(&mut db, &batch).unwrap(), Recording::Recorded);
        }
    }
    assert_eq!(
        read(&db, batch.conversation_id, batch.window_id, 0)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn completed_steer_suffix_is_allowed_but_other_or_incomplete_suffix_is_rejected() {
    for (kind, complete, allowed) in [
        (ItemKind::UserMessage, true, true),
        (ItemKind::UserMessage, false, false),
        (ItemKind::AssistantMessage, true, false),
    ] {
        let (mut db, mut batch) = setup();
        let item = Uuid::new_v4();
        append(
            &mut db,
            batch.conversation_id,
            7,
            EventPayload::ItemStarted {
                turn_id: batch.turn_id,
                item_id: item,
                kind,
            },
        );
        batch.boundary_seq = 8;
        if complete {
            append(
                &mut db,
                batch.conversation_id,
                8,
                EventPayload::ItemCompleted {
                    turn_id: batch.turn_id,
                    item_id: item,
                },
            );
            batch.boundary_seq = 9;
        }
        if allowed {
            assert_eq!(record(&mut db, &batch).unwrap(), Recording::Recorded);
            assert_eq!(
                read(&db, batch.conversation_id, batch.window_id, 0).unwrap()[0].anchor_item_id,
                batch.anchor_item_id
            );
        } else {
            assert_eq!(record(&mut db, &batch), Err(Error::Invalid));
        }
    }
}
