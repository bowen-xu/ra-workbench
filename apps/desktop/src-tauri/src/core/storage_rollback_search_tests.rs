//! Search visibility follows the production rollback projection; facts stay append-only.
use crate::{
    agent::repository::{SqliteConversationCatalog, journal},
    data_interface::{AuthorizeRollback, Database, SharedDatabase, journal::JournalCommand},
};
use research_agent_core::{
    conversation::{
        BindingKind, CatalogQuery, ConversationBinding, ConversationForkRequest,
        ConversationForkStore, ConversationId, ConversationSearch, ConversationSearchPage,
        ConversationSearchQuery, HistoryQuery,
    },
    protocol::{
        AgentEvent, ConversationProjection, EventPayload as P, ItemKind, ReasoningSummaryPart,
    },
};
use research_data_runtime::{WriteMode, WriteOptions};
use rusqlite::{Connection, params};
use std::path::Path;
use uuid::Uuid;

#[path = "storage_rollback_search_tests/upgrade.rs"]
mod upgrade;

const TITLE: &str = "retained-title-only";
const SUMMARY_ITEM: u128 = 30_100;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

struct Ledger {
    id: ConversationId,
    events: Vec<AgentEvent>,
    first_marker: usize,
    second_marker: usize,
}

fn ledger() -> Ledger {
    // Start with the actual Rust exporter fixture, then validate every added
    // synthetic fact through the same production projection before storing it.
    let exported: Vec<AgentEvent> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/agent-rollback-events-v1.json"
    ))
    .unwrap();
    let id = exported[0].conversation_id();
    let mut projection = ConversationProjection::new(id);
    let mut events = Vec::new();
    let mut markers = Vec::new();
    let mut emit = |payload| {
        let event = AgentEvent::new(id, events.len() as u64 + 1, 123, uuid(4), payload).unwrap();
        projection.apply(&event).unwrap();
        events.push(event);
    };
    for event in exported {
        if matches!(event.payload(), P::TurnFinished { turn_id, .. } if *turn_id == uuid(30)) {
            emit(P::ItemAccepted {
                turn_id: uuid(30),
                item_id: uuid(30_101),
                kind: ItemKind::AssistantMessage,
                text: "removed-assistant-needle".into(),
                phase: None,
                source: None,
            });
            let first = "removed-summary-needle🙂\0";
            let second = "separate-public-part";
            emit(P::ItemAccepted {
                turn_id: uuid(30),
                item_id: uuid(SUMMARY_ITEM),
                kind: ItemKind::ReasoningSummary {
                    parts: vec![
                        ReasoningSummaryPart {
                            index: 0.into(),
                            start_byte: 0,
                            len_byte: first.len(),
                        },
                        ReasoningSummaryPart {
                            index: 1.into(),
                            start_byte: first.len(),
                            len_byte: second.len(),
                        },
                    ],
                },
                text: format!("{first}{second}"),
                phase: None,
                source: None,
            });
        }
        let payload = match event.payload() {
            P::TextDelta {
                turn_id,
                item_id,
                text,
            } => P::TextDelta {
                turn_id: *turn_id,
                item_id: *item_id,
                text: format!(
                    "{text} {}",
                    if *turn_id == uuid(10) {
                        "surviving-user-needle"
                    } else if *turn_id == uuid(30) {
                        "removed-steer-needle"
                    } else {
                        "new-user-needle"
                    }
                ),
            },
            payload => payload.clone(),
        };
        let first_user_delta =
            matches!(&payload, P::TextDelta { turn_id, .. } if *turn_id == uuid(10));
        let compact = match &payload {
            P::TurnStarted {
                turn_id,
                task_kind: research_agent_core::protocol::TaskKind::Compact,
                ..
            } => Some(*turn_id),
            _ => None,
        };
        let marker = matches!(&payload, P::ConversationRolledBack { .. });
        emit(payload);
        if first_user_delta {
            // Cross the real history page limit and a journal batch boundary.
            for _ in 0..510 {
                emit(P::TextDelta {
                    turn_id: uuid(10),
                    item_id: uuid(10_000),
                    text: ".".into(),
                });
            }
        }
        if let Some(turn_id) = compact {
            let item_id = uuid(turn_id.as_u128() * 1000 + 99);
            emit(P::ItemStarted {
                turn_id,
                item_id,
                kind: ItemKind::ContextCompaction,
            });
            emit(P::ItemCompleted { turn_id, item_id });
        }
        if marker {
            markers.push(event.seq());
        }
    }
    // The marker sequence numbers in the exported source are independent of
    // the extra facts. Locate their actual new positions in the validated log.
    let positions = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            matches!(event.payload(), P::ConversationRolledBack { .. }).then_some(index)
        })
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 2);
    assert!(events.len() > 500);
    assert_eq!(projection.turn_order(), &[uuid(50)]);
    Ledger {
        id,
        events,
        first_marker: positions[0],
        second_marker: positions[1],
    }
}

fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("research-assistant.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("data-log")).unwrap()
}

fn seed_binding(sql: &Connection, id: ConversationId) -> ConversationBinding {
    sql.execute("insert into note_pages(id,title,created_at,updated_at) values('rollback-search-note','search fixture','now','now')", []).unwrap();
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,?2)",
        params![id.as_uuid().to_string(), TITLE],
    )
    .unwrap();
    sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','rollback-search-note')", [id.as_uuid().to_string()]).unwrap();
    ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "rollback-search-note".into(),
    }
}

fn authorize(database: &SharedDatabase, id: ConversationId) -> AuthorizeRollback {
    let binding = seed_binding(&database.lock().unwrap(), id);
    let command = AuthorizeRollback {
        operation_id: uuid(4),
        conversation_id: id,
        binding,
    };
    database.authorize_rollback(&command).unwrap();
    command
}

fn append(
    database: &SharedDatabase,
    events: &[AgentEvent],
    mode: WriteMode,
) -> Vec<JournalCommand> {
    let mut commands = Vec::new();
    let mut cursor = 0;
    while cursor < events.len() {
        // A rollback is one command, as required by the deployed journal codec.
        let end = if matches!(events[cursor].payload(), P::ConversationRolledBack { .. }) {
            cursor + 1
        } else {
            events[cursor..]
                .iter()
                .take(500)
                .position(|event| matches!(event.payload(), P::ConversationRolledBack { .. }))
                .map_or((cursor + 500).min(events.len()), |offset| cursor + offset)
        };
        let command = JournalCommand::new(
            events[cursor].conversation_id(),
            Uuid::new_v4(),
            events[cursor].seq() - 1,
            events[cursor..end].to_vec(),
            None,
            123,
        )
        .unwrap();
        database
            .write(
                command.clone(),
                WriteOptions {
                    mode,
                    ..Default::default()
                },
            )
            .unwrap();
        commands.push(command);
        cursor = end;
    }
    commands
}

fn history(sql: &mut Connection, id: ConversationId) -> Vec<AgentEvent> {
    let tx = sql.transaction().unwrap();
    let mut events = Vec::new();
    let mut offset = 0;
    let mut watermark = None;
    loop {
        let page = journal::read_history_in::<AgentEvent>(
            &tx,
            HistoryQuery::new(id, offset, 500).unwrap(),
        )
        .unwrap();
        let total = *watermark.get_or_insert(page.durable_count);
        assert_eq!(page.durable_count, total);
        for (index, event) in page.items.iter().enumerate() {
            assert_eq!(event.seq(), offset + index as u64 + 1);
        }
        events.extend(page.items);
        match page.next_offset {
            Some(next) => {
                assert_eq!(next, events.len() as u64);
                assert!(next > offset && next < total);
                offset = next;
            }
            None => {
                assert_eq!(events.len() as u64, total);
                break;
            }
        }
    }
    tx.commit().unwrap();
    events
}

fn assert_history(sql: &mut Connection, id: ConversationId, expected: &[AgentEvent]) {
    let actual = history(sql, id);
    assert_eq!(actual.len(), expected.len(), "full history length");
    let difference = actual
        .iter()
        .zip(expected)
        .position(|(one, two)| one != two);
    assert!(
        difference.is_none(),
        "first differing logical event: {difference:?}"
    );
}

fn rows(sql: &Connection, query: &str, id: ConversationId) -> Vec<String> {
    sql.prepare(query)
        .unwrap()
        .query_map([id.as_uuid().to_string()], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn preserved(sql: &Connection, id: ConversationId) -> Vec<Vec<String>> {
    [
        "select json_array(ordinal,schema_version,payload_json) from agent_journal_records where conversation_id=?1 order by ordinal",
        "select json_array(item_id,turn_id,role,body,completed,start_ordinal,last_ordinal) from agent_messages where conversation_id=?1 order by item_id",
        "select json_array(item_id,part_ordinal,summary_index,start_byte,len_byte) from agent_reasoning_parts where conversation_id=?1 order by item_id,part_ordinal",
        "select json_array(batch_id,expected_count,result_count,payload_hash) from agent_journal_batches where conversation_id=?1 order by batch_id",
    ].into_iter().map(|query| rows(sql, query, id)).collect()
}

fn assert_preserved(before: &[Vec<String>], after: &[Vec<String>]) {
    assert_eq!(before.len(), after.len());
    for (table, (before, after)) in before.iter().zip(after).enumerate() {
        assert!(
            before.iter().all(|row| after.contains(row)),
            "stored table {table} lost or rewrote a pre-existing row"
        );
    }
}

fn hidden(sql: &Connection, id: ConversationId) -> Vec<String> {
    rows(
        sql,
        "select turn_id from agent_rolled_back_turns where conversation_id=?1 order by turn_id",
        id,
    )
}

async fn search(database: &SharedDatabase, term: &str) -> ConversationSearchPage {
    SqliteConversationCatalog::new(database.clone())
        .search(ConversationSearchQuery {
            term: term.into(),
            scope: CatalogQuery {
                internal: false,
                binding: None,
                archived: false,
                limit: 10,
                before: None,
            },
        })
        .await
        .unwrap()
}

async fn assert_only_new(database: &SharedDatabase) {
    for term in [
        "surviving-user-needle",
        "removed-steer-needle",
        "removed-assistant-needle",
        "removed-summary-needle",
        "separate-public-part",
    ] {
        assert!(search(database, term).await.hits.is_empty(), "{term}");
    }
    let new = search(database, "new-user-needle").await;
    assert_eq!(new.hits.len(), 1);
    assert_eq!(new.hits[0].turn_id, Some(uuid(50)));
    let title = search(database, TITLE).await;
    assert_eq!(title.hits.len(), 1);
    assert_eq!(title.hits[0].item_id, None);
}

#[tokio::test]
async fn consecutive_user_rollbacks_hide_steers_compaction_and_summary_without_deleting_facts() {
    let root = tempfile::tempdir().unwrap();
    let ledger = ledger();
    let database = open(root.path());
    authorize(&database, ledger.id);
    append(
        &database,
        &ledger.events[..ledger.first_marker],
        WriteMode::Immediate,
    );
    let before = preserved(&database.lock().unwrap(), ledger.id);
    for term in [
        "removed-steer-needle",
        "removed-assistant-needle",
        "removed-summary-needle",
        "separate-public-part",
    ] {
        assert_eq!(
            search(&database, term).await.hits[0].turn_id,
            Some(uuid(30))
        );
    }
    let commands = append(
        &database,
        &ledger.events[ledger.first_marker..=ledger.first_marker],
        WriteMode::Automatic,
    );
    assert!(
        search(&database, "removed-summary-needle")
            .await
            .hits
            .is_empty()
    );
    assert_eq!(
        search(&database, "surviving-user-needle").await.hits[0].turn_id,
        Some(uuid(10))
    );
    assert_eq!(
        hidden(&database.lock().unwrap(), ledger.id),
        vec![uuid(30).to_string(), uuid(40).to_string()]
    );
    database
        .write(commands[0].clone(), WriteOptions::default())
        .unwrap();
    assert_eq!(hidden(&database.lock().unwrap(), ledger.id).len(), 2);
    append(
        &database,
        &ledger.events[ledger.second_marker..],
        WriteMode::Automatic,
    );
    assert_only_new(&database).await;
    {
        let mut sql = database.lock().unwrap();
        assert_eq!(
            hidden(&sql, ledger.id),
            [10, 20, 30, 40].map(|turn| uuid(turn).to_string())
        );
        assert_preserved(&before, &preserved(&sql, ledger.id));
        assert_history(&mut sql, ledger.id, &ledger.events);
    }
    database.shutdown().unwrap();
    drop(database);
    let database = open(root.path());
    assert_only_new(&database).await;
    assert_history(&mut database.lock().unwrap(), ledger.id, &ledger.events);
    database.shutdown().unwrap();
}

#[tokio::test]
async fn fork_through_new_turn_preserves_rollback_visibility_and_raw_parent_facts() {
    let root = tempfile::tempdir().unwrap();
    let ledger = ledger();
    let database = open(root.path());
    let command = authorize(&database, ledger.id);
    append(&database, &ledger.events, WriteMode::Immediate);
    let catalog = SqliteConversationCatalog::new(database.clone());
    let child = ConversationId::from_uuid(uuid(2));
    let request = ConversationForkRequest {
        child_id: child,
        parent_id: ledger.id,
        through_turn_id: uuid(50),
        binding: command.binding.clone(),
    };
    let fork = catalog.fork(request.clone()).await.unwrap();
    assert_eq!(fork.parent_end_seq, ledger.events.len() as u64);
    assert_eq!(catalog.fork(request).await.unwrap(), fork);
    for term in [
        "removed-steer-needle",
        "removed-assistant-needle",
        "removed-summary-needle",
        "surviving-user-needle",
    ] {
        assert!(search(&database, term).await.hits.is_empty(), "{term}");
    }
    let new = search(&database, "new-user-needle").await;
    assert_eq!(new.hits.len(), 2);
    assert!(new.hits.iter().any(|hit| hit.conversation.id == child));
    let mut sql = database.lock().unwrap();
    assert_eq!(hidden(&sql, child), hidden(&sql, ledger.id));
    assert_eq!(history(&mut sql, child).len(), ledger.events.len());
    assert_history(&mut sql, ledger.id, &ledger.events);
    drop(sql);
    // A historical fork before either marker must derive its own visibility,
    // rather than copying the current parent's accumulated tombstones.
    let earlier = ConversationId::from_uuid(uuid(3));
    catalog
        .fork(ConversationForkRequest {
            child_id: earlier,
            parent_id: ledger.id,
            through_turn_id: uuid(30),
            binding: command.binding,
        })
        .await
        .unwrap();
    assert!(hidden(&database.lock().unwrap(), earlier).is_empty());
    for term in [
        "removed-assistant-needle",
        "removed-summary-needle",
        "surviving-user-needle",
    ] {
        let result = search(&database, term).await;
        assert_eq!(result.hits.len(), 1, "{term}");
        assert_eq!(result.hits[0].conversation.id, earlier);
    }
    database.shutdown().unwrap();
}
