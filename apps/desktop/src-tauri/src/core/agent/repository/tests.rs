use super::*;
use research_agent_core::conversation::{ConversationId, HistoryQuery};
use rusqlite::Connection;
use uuid::Uuid;

fn id() -> ConversationId {
    ConversationId::from_uuid(Uuid::new_v4())
}

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 67);
    connection
}

fn history(connection: &mut Connection, id: ConversationId) -> Vec<String> {
    read_history(connection, HistoryQuery::new(id, 0, 500).unwrap())
        .unwrap()
        .items
}

#[test]
fn journal_survives_close_and_reopens_without_codex_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("research.sqlite3");
    let conversation = id();
    let batch = Uuid::new_v4();
    {
        let mut connection = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut connection, 67);
        create_conversation(&mut connection, conversation, "Paper").unwrap();
        append_batch(
            &mut connection,
            conversation,
            batch,
            0,
            &["first", "second"],
        )
        .unwrap();
    }
    let mut connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    assert_eq!(history(&mut connection, conversation), ["first", "second"]);
    assert_eq!(
        append_batch(
            &mut connection,
            conversation,
            batch,
            0,
            &["first", "second"]
        ),
        Ok(2)
    );
    assert_eq!(history(&mut connection, conversation).len(), 2);
}

#[test]
fn partial_insert_failure_rolls_back_items_batch_and_watermark() {
    let mut connection = database();
    let conversation = id();
    create_conversation(&mut connection, conversation, "Paper").unwrap();
    connection
        .execute_batch(
            "create trigger fail_second before insert on agent_items when new.ordinal = 1
         begin select raise(abort, 'injected write failure'); end;",
        )
        .unwrap();
    let batch = Uuid::new_v4();
    assert_eq!(
        append_batch(&mut connection, conversation, batch, 0, &["a", "b"]),
        Err(JournalError::Unavailable)
    );
    assert!(history(&mut connection, conversation).is_empty());
    let batches: i64 = connection
        .query_row("select count(*) from agent_journal_batches", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(batches, 0);
    connection
        .execute_batch("drop trigger fail_second;")
        .unwrap();
    assert_eq!(
        append_batch(&mut connection, conversation, batch, 0, &["a", "b"]),
        Ok(2)
    );
}

#[test]
fn retries_are_idempotent_and_stale_writers_cannot_append() {
    let mut connection = database();
    let conversation = id();
    create_conversation(&mut connection, conversation, "Paper").unwrap();
    let first = Uuid::new_v4();
    append_batch(&mut connection, conversation, first, 0, &["a"]).unwrap();
    append_batch(&mut connection, conversation, Uuid::new_v4(), 1, &["b"]).unwrap();
    assert_eq!(
        append_batch(&mut connection, conversation, first, 0, &["a"]),
        Ok(1)
    );
    assert_eq!(
        append_batch(&mut connection, conversation, first, 0, &["changed"]),
        Err(JournalError::BatchConflict)
    );
    assert_eq!(
        append_batch(&mut connection, conversation, Uuid::new_v4(), 0, &["stale"]),
        Err(JournalError::RevisionConflict)
    );
    assert_eq!(history(&mut connection, conversation), ["a", "b"]);
}

#[test]
fn pagination_is_bounded_and_conversations_are_isolated() {
    let mut connection = database();
    let first = id();
    let second = id();
    create_conversation(&mut connection, first, "One").unwrap();
    create_conversation(&mut connection, second, "Two").unwrap();
    append_batch(&mut connection, first, Uuid::new_v4(), 0, &["a", "b", "c"]).unwrap();
    append_batch(&mut connection, second, Uuid::new_v4(), 0, &["other"]).unwrap();
    let page =
        read_history::<String>(&mut connection, HistoryQuery::new(first, 1, 1).unwrap()).unwrap();
    assert_eq!(page.items, ["b"]);
    assert_eq!(page.next_offset, Some(2));
    assert_eq!(page.durable_count, 3);
    assert_eq!(history(&mut connection, second), ["other"]);
    assert!(
        read_history::<String>(&mut connection, HistoryQuery::new(first, 99, 1).unwrap())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn missing_item_and_invalid_payload_are_not_silently_skipped() {
    let mut connection = database();
    let conversation = id();
    create_conversation(&mut connection, conversation, "Paper").unwrap();
    append_batch(
        &mut connection,
        conversation,
        Uuid::new_v4(),
        0,
        &["a", "b"],
    )
    .unwrap();
    connection
        .execute(
            "update agent_items set payload_json = '123' where ordinal = 0",
            [],
        )
        .unwrap();
    assert_eq!(
        read_history::<String>(
            &mut connection,
            HistoryQuery::new(conversation, 0, 10).unwrap()
        ),
        Err(JournalError::Corrupt)
    );
    connection
        .execute("delete from agent_items where ordinal = 0", [])
        .unwrap();
    assert_eq!(
        read_history::<String>(
            &mut connection,
            HistoryQuery::new(conversation, 0, 10).unwrap()
        ),
        Err(JournalError::Corrupt)
    );
}

#[test]
fn validates_input_and_preserves_existing_conversation() {
    let mut connection = database();
    let conversation = id();
    assert_eq!(
        create_conversation(&mut connection, conversation, "  "),
        Err(JournalError::InvalidInput)
    );
    create_conversation(&mut connection, conversation, "Original").unwrap();
    assert_eq!(
        create_conversation(&mut connection, conversation, "Replacement"),
        Err(JournalError::AlreadyExists)
    );
    assert_eq!(
        append_batch::<String>(&mut connection, conversation, Uuid::new_v4(), 0, &[]),
        Err(JournalError::InvalidInput)
    );
    assert_eq!(
        append_batch(&mut connection, id(), Uuid::new_v4(), 0, &["missing"]),
        Err(JournalError::NotFound)
    );
    assert_eq!(
        append_batch(
            &mut connection,
            conversation,
            Uuid::new_v4(),
            u64::MAX,
            &["overflow"]
        ),
        Err(JournalError::InvalidInput)
    );
    let title: String = connection
        .query_row("select title from agent_conversations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(title, "Original");
}
