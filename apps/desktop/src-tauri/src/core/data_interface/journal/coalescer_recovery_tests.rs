//! Multi-key body writes and every receipt/control marker share the real SQL tx.
use super::coalescing::{count_bodies, delta, start};
use super::*;
use crate::data_interface::{Database, SharedDatabase};
use research_data_runtime::WriteOptions;
use rusqlite::functions::FunctionFlags;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn open(path: &Path) -> SharedDatabase {
    let mut db = Connection::open(path.join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    Database::open(db, &path.join("data-log")).unwrap()
}
fn messages(db: &Connection) -> Vec<(String, String, u64, bool)> {
    db.prepare("select conversation_id,body,last_ordinal,completed from agent_messages order by conversation_id").unwrap()
        .query_map([],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap()
}
fn prefix(db: &Connection) -> (u64, u64, u64, bool) {
    (
        db.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get(0),
        )
        .unwrap(),
        db.query_row("select count(*) from data_operation_receipts", [], |row| {
            row.get(0)
        })
        .unwrap(),
        db.query_row("select count(*) from agent_journal_records", [], |row| {
            row.get(0)
        })
        .unwrap(),
        db.query_row(
            "select coalesce_text from data_projection_control",
            [],
            |row| row.get(0),
        )
        .unwrap(),
    )
}

#[test]
fn tool_result_projection_failure_replays_accepted_fragments_and_receipt() {
    use research_agent_core::protocol::{
        ConversationProjection, ItemKind, ModelIdentity, TaskKind, TurnOutcome, TurnPhase,
    };
    let directory = tempfile::tempdir().unwrap();
    let database = open(directory.path());
    let conversation = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let attempt = Uuid::new_v4();
    let call_item = Uuid::new_v4();
    let call_id = Uuid::new_v4();
    let result_item = Uuid::new_v4();
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: None,
    };
    let first = "🦀".repeat(20_000);
    let second = "😀".repeat(20_000);
    let payloads = vec![
        EventPayload::TurnStarted {
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            task_kind: TaskKind::Regular,
            model: model.clone(),
        },
        EventPayload::PhaseChanged {
            turn_id: turn,
            phase: TurnPhase::RunningModel,
        },
        EventPayload::ModelRequestStarted {
            turn_id: turn,
            attempt_id: attempt,
            model: model.clone(),
        },
        EventPayload::ModelResponseStarted {
            turn_id: turn,
            attempt_id: attempt,
            response_id: "fixture".into(),
            model: Some(model),
        },
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: call_item,
            kind: ItemKind::ToolCall {
                call_id,
                name: "read".into(),
                provider_call_id: Some("fixture-call".into()),
                namespace: None,
            },
        },
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: call_item,
            text: "{}".into(),
        },
        EventPayload::ItemCompleted {
            turn_id: turn,
            item_id: call_item,
        },
        EventPayload::PhaseChanged {
            turn_id: turn,
            phase: TurnPhase::RunningTools,
        },
        EventPayload::ToolReceiptPending {
            turn_id: turn,
            call_id,
        },
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: result_item,
            kind: ItemKind::ToolResult {
                call_id,
                is_error: Some(false),
            },
        },
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: result_item,
            text: first.clone(),
        },
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: result_item,
            text: second.clone(),
        },
        EventPayload::ItemCompleted {
            turn_id: turn,
            item_id: result_item,
        },
        EventPayload::ToolReceiptConfirmed {
            turn_id: turn,
            call_id,
        },
        EventPayload::ModelRequestFinished {
            turn_id: turn,
            attempt_id: attempt,
            input_tokens: None,
            output_tokens: None,
        },
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
    ];
    let events: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(i, payload)| event(conversation, i as u64 + 1, payload))
        .collect();
    let initial = command(conversation, 0, events[..11].to_vec(), Some("tool fixture"));
    database.write(initial, WriteOptions::default()).unwrap();
    database.materialize().unwrap();
    let observer = Connection::open(directory.path().join("fixture.sqlite3")).unwrap();
    observer.execute_batch(&format!("create trigger fail_tool_fragment before insert on agent_journal_records when new.conversation_id='{}' and new.ordinal=11 begin select raise(abort,'synthetic tool projection failure'); end;", conversation.as_uuid())).unwrap();
    let before = prefix(&observer);
    let tail = command(conversation, 11, events[11..].to_vec(), None);
    let receipt = database
        .write(tail.clone(), WriteOptions::default())
        .unwrap();
    assert!(database.materialize().is_err());
    let count: u64 = observer
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [conversation.as_uuid().to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 11);
    assert_eq!(prefix(&observer), before);
    assert!(database.lock().is_err());
    drop(database);
    observer
        .execute_batch("drop trigger fail_tool_fragment")
        .unwrap();
    let recovered = open(directory.path());
    let saved = storage::read_history::<AgentEvent>(
        &mut recovered.lock().unwrap(),
        HistoryQuery::new(conversation, 0, 500).unwrap(),
    )
    .unwrap()
    .items;
    assert_eq!(saved, events);
    let mut projection = ConversationProjection::new(conversation);
    for saved_event in &saved {
        projection.apply(saved_event).unwrap();
    }
    let item = &projection.turn(turn).unwrap().items[&result_item];
    assert!(item.completed && item.text == format!("{first}{second}"));
    assert!(!projection.has_unknown_tool_outcome());
    let changes = recovered.lock().unwrap().total_changes();
    let replay = recovered.write(tail, WriteOptions::default()).unwrap();
    assert!(replay.replayed);
    assert_eq!(
        (replay.epoch, replay.position, replay.operation_id),
        (receipt.epoch, receipt.position, receipt.operation_id)
    );
    assert_eq!(recovered.lock().unwrap().total_changes(), changes);
    recovered.shutdown().unwrap();
}

#[test]
fn receipt_and_marker_failures_after_multi_key_flush_rollback_then_replay_both_engines_exactly() {
    for fail_receipt in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let database = open(directory.path());
        let a = ConversationId::from_uuid(Uuid::new_v4());
        let b = ConversationId::from_uuid(Uuid::new_v4());
        let ta = Uuid::new_v4();
        let tb = Uuid::new_v4();
        let ia = Uuid::new_v4();
        let ib = Uuid::new_v4();
        let setup = [
            command(a, 0, vec![start(a, 1, ta, ia)], Some("A")),
            command(b, 0, vec![start(b, 1, tb, ib)], Some("B")),
        ];
        let mut originals = vec![];
        for command in setup {
            let receipt = database
                .write(command.clone(), WriteOptions::default())
                .unwrap();
            originals.push((command, receipt));
        }
        database.materialize().unwrap();
        let flushed = Arc::new(AtomicBool::new(false));
        let probe = flushed.clone();
        {
            let db = database.lock().unwrap();
            count_bodies(&db);
            // This test-only callback observes the real trigger's pre-failure
            // SQL state; unlike SQLite rows, its evidence survives rollback.
            db.create_scalar_function(
                "observe_multi_key_flush",
                3,
                FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS,
                move |context| {
                    let count: u64 = context.get(0)?;
                    let first: String = context.get(1)?;
                    let second: String = context.get(2)?;
                    probe.store(
                        count == 2 && first == "A1A2" && second == "B1",
                        Ordering::Release,
                    );
                    Ok(1)
                },
            )
            .unwrap();
        }
        let observer = Connection::open(directory.path().join("fixture.sqlite3")).unwrap();
        let (timing, table, condition) = if fail_receipt {
            ("insert", "data_operation_receipts", "new.position=6")
        } else {
            (
                "update of materialized_position",
                "data_runtime_state",
                "new.materialized_position=6",
            )
        };
        observer.execute_batch(&format!("create trigger fail_after_coalesced_bodies before {timing} on {table} when {condition} begin
            select observe_multi_key_flush((select count(*) from body_updates),
                (select body from agent_messages where conversation_id='{}' and item_id='{}'),
                (select body from agent_messages where conversation_id='{}' and item_id='{}'));
            select raise(abort,'synthetic failure after actual multi-key flush'); end;",a.as_uuid(),ia,b.as_uuid(),ib)).unwrap();
        let before = prefix(&observer);
        let before_messages = messages(&observer);
        for command in [
            command(a, 1, vec![delta(a, 2, ta, ia, "A1")], None),
            command(b, 1, vec![delta(b, 2, tb, ib, "B1")], None),
            command(a, 2, vec![delta(a, 3, ta, ia, "A2")], None),
            command(
                a,
                3,
                vec![event(
                    a,
                    4,
                    EventPayload::ItemCompleted {
                        turn_id: ta,
                        item_id: ia,
                    },
                )],
                None,
            ),
        ] {
            let receipt = database
                .write(command.clone(), WriteOptions::default())
                .unwrap();
            originals.push((command, receipt));
        }
        assert!(database.materialize().is_err());
        assert!(
            flushed.load(Ordering::Acquire),
            "failure must follow both actual UPDATEs"
        );
        assert_eq!(prefix(&observer), before);
        assert_eq!(messages(&observer), before_messages);
        assert_eq!(
            observer
                .query_row("select count(*) from body_updates", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(before, (2, 2, 2, false));
        // A failed prefix cannot accept another valid new conversation. Retain
        // the accepted originals for cold recovery without allocating replay IDs.
        assert!(
            database
                .write(
                    command(
                        ConversationId::from_uuid(Uuid::new_v4()),
                        0,
                        vec![],
                        Some("blocked")
                    ),
                    WriteOptions::default()
                )
                .is_err()
        );
        drop(database);
        observer
            .execute_batch("drop trigger fail_after_coalesced_bodies")
            .unwrap();
        let recovered = open(directory.path());
        {
            let mut db = recovered.lock().unwrap();
            assert_eq!(prefix(&db), (6, 6, 6, false));
            let mut expected = vec![
                (a.as_uuid().to_string(), "A1A2".into(), 3, true),
                (b.as_uuid().to_string(), "B1".into(), 1, false),
            ];
            expected.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(messages(&db), expected);
            assert_eq!(
                db.query_row("select count(*) from body_updates", [], |row| row
                    .get::<_, u64>(0))
                    .unwrap(),
                2
            );
            for id in [a, b] {
                let expected: Vec<_> = originals
                    .iter()
                    .filter(|(command, _)| command.conversation_id == *id.as_uuid())
                    .flat_map(|(command, _)| command.events.iter().cloned())
                    .collect();
                assert_eq!(
                    storage::read_history::<AgentEvent>(
                        &mut db,
                        HistoryQuery::new(id, 0, 500).unwrap()
                    )
                    .unwrap()
                    .items,
                    expected
                );
            }
        }
        let changes = recovered.lock().unwrap().total_changes();
        for (command, original) in originals {
            let receipt = recovered.write(command, WriteOptions::default()).unwrap();
            assert!(receipt.replayed);
            assert_eq!(
                (
                    receipt.epoch,
                    receipt.operation_id,
                    receipt.position,
                    receipt.result
                ),
                (
                    original.epoch,
                    original.operation_id,
                    original.position,
                    original.result
                )
            );
        }
        assert_eq!(recovered.lock().unwrap().total_changes(), changes);
        recovered.shutdown().unwrap();
    }
}
