use super::*;
use crate::data_interface::{Database, journal::JournalCommand};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use research_data_runtime::{RaftLog, coordinator::WriteOptions};
use rusqlite::params;

fn function(connection: &Connection, conversation: &str, batch: &str) -> rusqlite::Result<String> {
    connection.query_row(
        "select data_journal_operation_id(?1,?2)",
        params![conversation, batch],
        |row| row.get(0),
    )
}

fn sql_receipt(connection: &Connection, operation: Uuid) -> rusqlite::Result<usize> {
    connection.execute(
        "insert into data_sql_operation_receipts values(?1,?2,'identity.fixture',1,?3,0,'null')",
        params![
            operation.to_string(),
            Uuid::new_v4().to_string(),
            [7_u8; 32].as_slice()
        ],
    )
}

#[test]
fn pure_uuid_function_is_registered_per_connection_and_preserves_parse_compatibility() {
    let first = Connection::open_in_memory().unwrap();
    let second = Connection::open_in_memory().unwrap();
    let conversation = Uuid::new_v4();
    let batch = Uuid::new_v4();
    assert!(function(&first, &conversation.to_string(), &batch.to_string()).is_err());
    register(&first).unwrap();
    let expected = Uuid::new_v5(&conversation, batch.as_bytes()).to_string();
    for (c, b) in [
        (conversation.to_string(), batch.to_string()),
        (
            conversation.to_string().to_uppercase(),
            batch.simple().to_string(),
        ),
        (conversation.urn().to_string(), batch.braced().to_string()),
    ] {
        assert_eq!(function(&first, &c, &b).unwrap(), expected);
    }
    assert!(function(&second, &conversation.to_string(), &batch.to_string()).is_err());
    register(&second).unwrap();
    assert_eq!(
        function(&second, &conversation.to_string(), &batch.to_string()).unwrap(),
        expected
    );
    let flags: u32 = first.query_row(
        "select flags from pragma_function_list where name='data_journal_operation_id' and narg=2",
        [], |row| row.get(0),
    ).unwrap();
    assert_ne!(flags & FunctionFlags::SQLITE_DETERMINISTIC.bits() as u32, 0);
    assert_ne!(flags & FunctionFlags::SQLITE_INNOCUOUS.bits() as u32, 0);
    assert_eq!(flags & FunctionFlags::SQLITE_DIRECTONLY.bits() as u32, 0);
}

#[test]
fn invalid_uuid_inputs_fail_without_disclosing_values() {
    let db = Connection::open_in_memory().unwrap();
    register(&db).unwrap();
    let valid = Uuid::new_v4().to_string();
    let secret_fixture = "synthetic-invalid-private-identity";
    for invalid in [secret_fixture, "", "00000000-0000-0000-0000-000000000000"] {
        for (c, b) in [(invalid, valid.as_str()), (valid.as_str(), invalid)] {
            let error = function(&db, c, b).unwrap_err().to_string();
            assert!(error.contains("DATA_JOURNAL_IDENTITY_INVALID"));
            assert!(!error.contains(secret_fixture));
        }
    }
    for sql in [
        "select data_journal_operation_id(null,?1)",
        "select data_journal_operation_id(42,?1)",
        "select data_journal_operation_id(x'ff',?1)",
    ] {
        let error = db
            .query_row(sql, [&valid], |row| row.get::<_, String>(0))
            .unwrap_err();
        assert!(error.to_string().contains("DATA_JOURNAL_IDENTITY_INVALID"));
    }
}

#[test]
fn trusted_schema_off_keeps_identity_triggers_atomic_and_tombstones_survive_delete() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute_batch("pragma trusted_schema=off").unwrap();
    let c = Uuid::new_v4();
    let b = Uuid::new_v4();
    let operation = Uuid::new_v5(&c, b.as_bytes());
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'identity')",
        [c.to_string()],
    )
    .unwrap();
    db.execute(
        "insert into agent_journal_batches values(?1,?2,0,1,'legacy-hash')",
        params![c.to_string(), b.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "select operation_id from data_journal_operation_ids",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        operation.to_string()
    );
    assert!(sql_receipt(&db, operation).is_err());
    db.execute(
        "delete from agent_conversations where id=?1",
        [c.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_journal_batches", [], |row| row
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from data_journal_operation_ids",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert!(sql_receipt(&db, operation).is_err());
    // Restoring the original batch retains the same evidence, rather than
    // allocating another operation or deleting the retained namespace entry.
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'restored')",
        [c.to_string()],
    )
    .unwrap();
    db.execute(
        "insert into agent_journal_batches values(?1,?2,0,1,'legacy-hash')",
        params![c.to_string(), b.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from data_journal_operation_ids",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    let foreign_keys: u64 = db
        .query_row(
            "select count(*) from pragma_foreign_key_list('data_journal_operation_ids')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(foreign_keys, 0);
}

fn event(c: ConversationId, request: Uuid, seq: u64) -> AgentEvent {
    AgentEvent::new(
        c,
        seq,
        seq,
        request,
        EventPayload::ItemAccepted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
            text: "identity fixture".into(),
            phase: None,
            source: None,
        },
    )
    .unwrap()
}

#[test]
fn sql_collision_rejects_before_log_accept_but_raw_batch_and_request_are_separate_domains() {
    let root = tempfile::tempdir().unwrap();
    let sql_path = root.path().join("fixture.sqlite3");
    let log_path = root.path().join("data-log");
    let mut sql = Connection::open(&sql_path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open(sql, &log_path).unwrap();
    let epoch: String = database
        .lock()
        .unwrap()
        .query_row("select epoch from data_runtime_state", [], |row| row.get(0))
        .unwrap();
    let epoch = Uuid::parse_str(&epoch).unwrap();
    let rejected_c = ConversationId::from_uuid(Uuid::new_v4());
    let rejected_b = Uuid::new_v4();
    let rejected_operation = Uuid::new_v5(rejected_c.as_uuid(), rejected_b.as_bytes());
    let request = Uuid::new_v4();
    {
        let sql = database.lock().unwrap();
        sql_receipt(&sql, rejected_operation).unwrap();
    }
    let rejected = JournalCommand::new(
        rejected_c,
        rejected_b,
        0,
        vec![event(rejected_c, request, 1)],
        Some("rejected".into()),
        1,
    )
    .unwrap();
    assert!(database.write(rejected, WriteOptions::default()).is_err());
    let batch = Uuid::new_v4();
    {
        let sql = database.lock().unwrap();
        sql_receipt(&sql, batch).unwrap();
        sql_receipt(&sql, request).unwrap();
    }
    // The same raw B is legal in two different C namespaces. R is an event's
    // business identity; neither B nor R is the journal's global operation ID.
    for position in 1..=2 {
        let c = ConversationId::from_uuid(Uuid::new_v4());
        let command = JournalCommand::new(
            c,
            batch,
            0,
            vec![event(c, request, 1)],
            Some("accepted".into()),
            1,
        )
        .unwrap();
        let receipt = database.write(command, WriteOptions::default()).unwrap();
        assert_eq!(receipt.position, position);
        assert_eq!(
            receipt.operation_id,
            Uuid::new_v5(c.as_uuid(), batch.as_bytes())
        );
        assert_eq!(receipt.epoch, epoch);
    }
    {
        let sql = database.lock().unwrap();
        assert_eq!(
            sql.query_row("select count(*) from agent_conversations", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            sql.query_row(
                "select count(*) from data_journal_operation_ids",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            2
        );
    }
    database.shutdown().unwrap();
    drop(database);
    let log = RaftLog::open(&log_path, Some(epoch)).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 2);
}
