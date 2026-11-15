//! Storage-boundary regressions use real journal commands and both engines.
//! Native decoder/service composition is verified by the model integration tests.
use super::*;
use crate::{
    agent::repository::{SqliteConversationCatalog, journal},
    data_interface::journal::JournalCommand,
};
use research_agent_core::{
    conversation::{
        CatalogQuery, ConversationId, ConversationSearch, ConversationSearchQuery, HistoryQuery,
    },
    protocol::{AgentEvent, EventPayload as P, ItemKind},
};
use research_data_runtime::{RaftLog, WriteMode, WriteOptions};
use serde_json::{Value, json};
use uuid::Uuid;

#[path = "storage_reasoning_performance_tests.rs"]
mod performance;
#[path = "storage_reasoning_search_tests.rs"]
mod search;
#[path = "storage_reasoning_upgrade_tests.rs"]
mod upgrade;

const OLD_BODY: &str = "旧🙂\0\n保真";
type ReceiptRow = (String, String, u64, Vec<u8>, String);

fn payload(value: Value) -> P {
    serde_json::from_value(value).unwrap()
}
fn summary_kind(parts: Value) -> ItemKind {
    serde_json::from_value(json!({"type":"reasoningSummary","parts":parts})).unwrap()
}
fn part(index: &str, start: usize, length: usize) -> Value {
    json!({"index":index,"startByte":start,"lenByte":length})
}
fn delta(turn: Uuid, item: Uuid, index: &str, text: &str) -> P {
    payload(
        json!({"type":"reasoningSummaryDelta","turnId":turn,"itemId":item,
        "summaryIndex":index,"text":text}),
    )
}
fn added(turn: Uuid, item: Uuid, index: &str) -> P {
    payload(
        json!({"type":"reasoningSummaryPartAdded","turnId":turn,"itemId":item,
        "summaryIndex":index}),
    )
}
fn accepted(turn: Uuid, item: Uuid, text: &str, parts: Value) -> P {
    P::ItemAccepted {
        turn_id: turn,
        item_id: item,
        kind: summary_kind(parts),
        text: text.into(),
        phase: None,
        source: None,
    }
}
fn events(id: ConversationId, after: u64, payloads: Vec<P>) -> Vec<AgentEvent> {
    payloads
        .into_iter()
        .enumerate()
        .map(|(i, payload)| {
            AgentEvent::new(id, after + i as u64 + 1, 0, Uuid::from_u128(42), payload).unwrap()
        })
        .collect()
}
fn write(
    database: &SharedDatabase,
    id: ConversationId,
    after: u64,
    payloads: Vec<P>,
    title: Option<&str>,
    mode: WriteMode,
) -> Vec<AgentEvent> {
    let events = events(id, after, payloads);
    let command = JournalCommand::new(
        id,
        Uuid::new_v4(),
        after,
        events.clone(),
        title.map(str::to_owned),
        1_790_000_000_000,
    )
    .unwrap();
    database
        .write(
            command,
            WriteOptions {
                mode,
                ..WriteOptions::default()
            },
        )
        .unwrap();
    events
}
fn history(sql: &mut Connection, id: ConversationId) -> Vec<AgentEvent> {
    journal::read_history(sql, HistoryQuery::new(id, 0, 500).unwrap())
        .unwrap()
        .items
}
fn open_logged(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("research-assistant.sqlite3")).unwrap();
    migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("data-log")).unwrap()
}
fn physical(sql: &Connection, id: ConversationId) -> Vec<(u64, String)> {
    sql.prepare("select ordinal,payload_json from agent_journal_records where conversation_id=?1 order by ordinal")
        .unwrap()
        .query_map([id.as_uuid().to_string()], |row| Ok((row.get(0)?,row.get(1)?)))
        .unwrap().collect::<Result<_, _>>().unwrap()
}
fn receipts(sql: &Connection) -> Vec<ReceiptRow> {
    sql.prepare("select operation_id,epoch,position,fingerprint,result_json from data_operation_receipts order by position")
        .unwrap().query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))
        .unwrap().collect::<Result<_, _>>().unwrap()
}
fn assert_fk(sql: &Connection) {
    assert_eq!(
        sql.pragma_query_value::<u32, _>(None, "foreign_keys", |r| r.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        sql.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn summary_deltas_coalesce_then_reopen_and_search_as_public_text() {
    let root = tempfile::tempdir().unwrap();
    let database = open_logged(root.path());
    let observer = Connection::open(root.path().join("research-assistant.sqlite3")).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    let index = "9007199254740993";
    let mut expected = write(
        &database,
        id,
        0,
        vec![
            P::ItemStarted {
                turn_id: turn,
                item_id: item,
                kind: summary_kind(json!([])),
            },
            added(turn, item, index),
        ],
        Some("summary fixture"),
        WriteMode::Immediate,
    );
    observer.execute_batch("create table summary_updates(value integer); insert into summary_updates values(0);
        create trigger count_summary_updates after update of body on agent_messages begin update summary_updates set value=value+1; end;").unwrap();
    let text = "needle你🙂\0".repeat(128);
    let mut tail = (0..128)
        .map(|_| delta(turn, item, index, "needle你🙂\0"))
        .collect::<Vec<_>>();
    tail.push(accepted(
        turn,
        item,
        &text,
        json!([part(index, 0, text.len())]),
    ));
    expected.extend(write(&database, id, 2, tail, None, WriteMode::Automatic));
    assert_eq!(
        observer
            .query_row("select value from summary_updates", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
    {
        let mut sql = database.lock().unwrap();
        assert_eq!(history(&mut sql, id), expected);
        assert_eq!(
            sql.query_row(
                "select role,body,completed from agent_messages where item_id=?1",
                [item.to_string()],
                |r| Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?
                ))
            )
            .unwrap(),
            ("reasoningSummary".into(), text, true)
        );
        assert_eq!(
            sql.query_row("select value from summary_updates", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(sql.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='storedReasoningSlice'",[],|r|r.get::<_,u32>(0)).unwrap(),128);
        assert_fk(&sql);
    }
    let catalog = SqliteConversationCatalog::new(database.clone());
    let result = catalog
        .search(ConversationSearchQuery {
            term: "needle".into(),
            scope: CatalogQuery {
                internal: false,
                binding: None,
                archived: false,
                limit: 10,
                before: None,
            },
        })
        .await
        .unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].item_id, Some(item));
    assert!(result.hits[0].completed);
    drop(catalog);
    database.shutdown().unwrap();
    drop(database);
    let database = open_logged(root.path());
    assert_eq!(history(&mut database.lock().unwrap(), id), expected);
    database.shutdown().unwrap();
}

#[test]
fn summary_part_byte_proofs_preserve_interleaving_and_replaced_drafts() {
    let mut sql = Connection::open_in_memory().unwrap();
    migrate(&mut sql).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    journal::create_conversation(&mut sql, id, "summary compatibility").unwrap();
    let item = Uuid::new_v4();
    let final_text = "你\0🙂tail";
    let mut payloads = vec![
        P::ItemStarted {
            turn_id: turn,
            item_id: item,
            kind: summary_kind(json!([])),
        },
        added(turn, item, "0"),
        delta(turn, item, "0", "你\0"),
        added(turn, item, "9223372036854775807"),
        delta(turn, item, "9223372036854775807", "tail"),
        delta(turn, item, "0", "🙂"),
        accepted(
            turn,
            item,
            final_text,
            json!([part("0", 0, 8), part("9223372036854775807", 8, 4),]),
        ),
    ];
    let replacement = Uuid::new_v4();
    payloads.extend([
        P::ItemStarted {
            turn_id: turn,
            item_id: replacement,
            kind: summary_kind(json!([])),
        },
        delta(turn, replacement, "-1", "draft"),
        accepted(turn, replacement, "final", json!([part("-1", 0, 5)])),
    ]);
    let partial = Uuid::new_v4();
    payloads.extend([
        P::ItemStarted {
            turn_id: turn,
            item_id: partial,
            kind: summary_kind(json!([])),
        },
        delta(turn, partial, "0", "same"),
        delta(turn, partial, "0", "wrong"),
        accepted(turn, partial, "samefixed", json!([part("0", 0, 9)])),
    ]);
    let empty = Uuid::new_v4();
    payloads.extend([
        P::ItemStarted {
            turn_id: turn,
            item_id: empty,
            kind: summary_kind(json!([])),
        },
        delta(turn, empty, "0", "discard"),
        accepted(turn, empty, "", json!([])),
    ]);
    let sparse = Uuid::new_v4();
    payloads.push(accepted(turn, sparse, "done", json!([part("5", 0, 4)])));
    let expected = events(id, 0, payloads);
    journal::append_batch(&mut sql, id, Uuid::new_v4(), 0, &expected).unwrap();
    assert_eq!(history(&mut sql, id), expected);
    for (item, text) in [
        (item, final_text),
        (replacement, "final"),
        (partial, "samefixed"),
        (empty, ""),
        (sparse, "done"),
    ] {
        assert_eq!(
            sql.query_row(
                "select body from agent_messages where item_id=?1",
                [item.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            text
        );
    }
    let types = |item: Uuid| {
        sql.prepare("select json_extract(payload_json,'$.payload.type') from agent_journal_records where json_extract(payload_json,'$.payload.itemId')=?1 and json_extract(payload_json,'$.payload.summaryIndex') is not null order by ordinal")
        .unwrap().query_map([item.to_string()],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
    };
    assert_eq!(
        types(item),
        [
            "reasoningSummaryPartAdded",
            "storedReasoningSlice",
            "reasoningSummaryPartAdded",
            "storedReasoningSlice",
            "storedReasoningSlice"
        ]
    );
    assert_eq!(types(replacement), ["reasoningSummaryDelta"]);
    assert_eq!(
        types(partial),
        ["storedReasoningSlice", "reasoningSummaryDelta"]
    );
    assert_eq!(types(empty), ["reasoningSummaryDelta"]);
    assert_fk(&sql);
}
