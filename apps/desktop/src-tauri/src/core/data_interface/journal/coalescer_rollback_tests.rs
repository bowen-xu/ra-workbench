//! Rollback preprocessing remains before the non-delta body barrier, atomically.
use super::*;
use super::{
    coalescing::{count_bodies, delta, start},
    coalescing_recovery::open,
};
use crate::{
    agent::repository::{context_windows, input_queue},
    data_interface::SharedDatabase,
};
use research_data_runtime::WriteOptions;
use rusqlite::functions::FunctionFlags;
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "coalescer_rollback_tests/fixture.rs"]
mod fixture;

fn observe_order(
    database: &SharedDatabase,
    fixture: &fixture::Fixture,
    position: u64,
    fail_receipt: bool,
) -> Arc<AtomicUsize> {
    let observed = Arc::new(AtomicUsize::new(0));
    let captured = observed.clone();
    let db = database.lock().unwrap();
    count_bodies(&db);
    db.create_scalar_function(
        "observe_rollback_body_order",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS,
        move |context| {
            let stage: usize = context.get(0)?;
            let correct: bool = context.get(1)?;
            if correct {
                captured.fetch_or(stage, Ordering::AcqRel);
            }
            Ok(1)
        },
    )
    .unwrap();
    let a = fixture.a.as_uuid();
    let b = fixture.b.as_uuid();
    let first = fixture.first.window_id;
    let empty = format!(
        "(select count(*) from body_updates)=0 and (select count(*) from agent_messages where conversation_id='{b}' and body='')=2"
    );
    db.execute_batch(&format!("create trigger observe_window_preprocess before update on agent_active_context_windows
        when new.conversation_id='{a}' begin select observe_rollback_body_order(1,new.window_id='{first}' and {empty}); end;
        create trigger observe_queue_preprocess before update on agent_input_queues when new.conversation_id='{a}' begin
        select observe_rollback_body_order(2,new.revision={} and {empty}
            and (select window_id from agent_active_context_windows where conversation_id='{a}')='{first}'
            and (select count(*) from agent_compact_session_start where conversation_id='{a}')=0); end;
        create trigger observe_rollback_insert before insert on agent_journal_records
        when new.conversation_id='{a}' and json_extract(new.payload_json,'$.payload.type')='conversationRolledBack' begin
        select observe_rollback_body_order(4,(select count(*) from body_updates)=2
            and (select count(*) from agent_messages where conversation_id='{b}' and body!='')=2
            and (select window_id from agent_active_context_windows where conversation_id='{a}')='{first}'
            and (select revision from agent_input_queues where conversation_id='{a}')={}
            and (select count(*) from agent_rolled_back_turns where conversation_id='{a}')=1); end;",
        fixture.queue_revision+1,fixture.queue_revision+1)).unwrap();
    let (action, table, predicate) = if fail_receipt {
        (
            "insert",
            "data_operation_receipts",
            format!("new.position={}", position + 4),
        )
    } else {
        (
            "update of materialized_position",
            "data_runtime_state",
            format!("new.materialized_position={}", position + 5),
        )
    };
    db.execute_batch(&format!("create trigger reject_rollback_transaction before {action} on {table} when {predicate} begin
        select observe_rollback_body_order(8,(select count(*) from body_updates)>={}
            and (select window_id from agent_active_context_windows where conversation_id='{a}')='{first}'
            and (select revision from agent_input_queues where conversation_id='{a}')={});
        select raise(abort,'synthetic failure after rollback preprocessing and multi-key flush'); end;",
        if fail_receipt {2}else{3},fixture.queue_revision+1)).unwrap();
    observed
}

#[test]
fn ended_conversation_rollback_with_other_pending_keys_is_atomic_and_cold_replays_exactly() {
    for fail_receipt in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let database = open(directory.path());
        let mut fixture = fixture::setup(&database);
        let observer = Connection::open(directory.path().join("fixture.sqlite3")).unwrap();
        let position = database
            .lock()
            .unwrap()
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap();
        let observed = observe_order(&database, &fixture, position, fail_receipt);
        let before = fixture::snapshot(&observer);
        for command in fixture.suffix.clone() {
            let receipt = database
                .write(command.clone(), WriteOptions::default())
                .unwrap();
            fixture.originals.push((command, receipt));
        }
        // Admission itself has not run the rollback preprocessor or body SQL.
        assert_eq!(fixture::snapshot(&observer), before);
        assert_eq!(observed.load(Ordering::Acquire), 0);
        assert!(database.materialize().is_err());
        assert_eq!(
            observed.load(Ordering::Acquire),
            15,
            "window/queue preprocessing, barrier and fault must all execute in order"
        );
        assert_eq!(fixture::snapshot(&observer), before);
        assert_eq!(
            observer
                .query_row("select count(*) from body_updates", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        drop(database);
        observer
            .execute_batch(
                "drop trigger observe_window_preprocess;drop trigger observe_queue_preprocess;
            drop trigger observe_rollback_insert;drop trigger reject_rollback_transaction;",
            )
            .unwrap();
        let recovered = open(directory.path());
        {
            let mut db = recovered.lock().unwrap();
            assert_eq!(
                context_windows::read_active(&db, *fixture.a.as_uuid())
                    .unwrap()
                    .unwrap()
                    .window_id,
                fixture.first.window_id
            );
            let queue =
                input_queue::read_checked(&mut db, *fixture.a.as_uuid(), |_| Ok(())).unwrap();
            assert_eq!(queue.revision(), fixture.queue_revision + 1);
            assert!(queue.paused());
            assert_eq!(queue.preparing(), Some(fixture.head));
            assert!(queue.input_paused(fixture.tail));
            assert_eq!(
                queue
                    .inputs()
                    .iter()
                    .map(|input| input.id)
                    .collect::<Vec<_>>(),
                vec![fixture.head, fixture.tail]
            );
            assert_eq!(
                db.query_row(
                    "select count(*) from agent_compact_session_start where conversation_id=?1",
                    [fixture.a.as_uuid().to_string()],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(db.query_row("select rollback_ordinal from agent_rolled_back_turns where conversation_id=?1 and turn_id=?2",params![fixture.a.as_uuid().to_string(),fixture.second.turn_id.to_string()],|row|row.get::<_,u64>(0)).unwrap(),10);
            assert_eq!(
                db.query_row(
                    "select position from data_operation_receipts where operation_id=?1",
                    [fixture.rollback.operation().to_string()],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
                position + 4
            );
            for (index, text, last) in [(0, "你\0🙂", 5), (1, "B1B2", 6)] {
                assert_eq!(db.query_row("select body,last_ordinal,completed from agent_messages where conversation_id=?1 and item_id=?2",
                    params![fixture.b.as_uuid().to_string(),fixture.items[index].to_string()],|row|Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?,row.get::<_,bool>(2)?))).unwrap(),(text.into(),last,false));
            }
            assert_eq!(
                db.query_row("select count(*) from body_updates", [], |row| row
                    .get::<_, u64>(0))
                    .unwrap(),
                3
            );
            for id in [fixture.a, fixture.b] {
                let expected: Vec<_> = fixture
                    .originals
                    .iter()
                    .filter(|(command, _)| command.conversation_id == *id.as_uuid())
                    .flat_map(|(command, _)| command.events.iter().cloned())
                    .collect();
                let history = storage::read_history::<AgentEvent>(
                    &mut db,
                    HistoryQuery::new(id, 0, 500).unwrap(),
                )
                .unwrap();
                assert_eq!(history.items, expected);
                let mut projection = research_agent_core::protocol::ConversationProjection::new(id);
                for event in &history.items {
                    projection.apply(event).unwrap();
                }
                if id == fixture.a {
                    assert!(projection.turn(fixture.second.turn_id).is_none());
                    assert!(projection.turn(fixture.first.turn_id).is_some());
                }
            }
        }
        let after = fixture::snapshot(&recovered.lock().unwrap());
        let changes = recovered.lock().unwrap().total_changes();
        for (command, original) in fixture.originals {
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
        assert_eq!(fixture::snapshot(&recovered.lock().unwrap()), after);
        assert_eq!(recovered.lock().unwrap().total_changes(), changes);
        recovered.shutdown().unwrap();
    }
}
