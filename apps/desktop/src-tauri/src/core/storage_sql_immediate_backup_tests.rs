use super::*;
use crate::{
    agent::repository::journal as journal_storage,
    data_interface::{AuthorizeRollback, Database, SqlImmediateStatus, journal::JournalCommand},
};
use research_agent_core::{
    conversation::{BindingKind, ConversationBinding, ConversationId, HistoryQuery},
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use research_data_runtime::{RaftLog, WriteOptions};
use rusqlite::params;
use uuid::Uuid;

type ReceiptRow = (String, String, String, u32, Vec<u8>, u64, String);

fn receipt_row(sql: &Connection, operation: Uuid) -> ReceiptRow {
    sql.query_row(
        "select operation_id,epoch,kind,version,fingerprint,sql_fence_position,result_json
         from data_sql_operation_receipts where operation_id=?1",
        [operation.to_string()],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )
    .unwrap()
}

#[test]
fn zero_fence_sql_receipt_survives_paired_backup_replay_and_exact_retry_without_a_log_position() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("research-assistant.sqlite3");
    let log_path = root.path().join("data-log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open(sql, &log_path).unwrap();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let note = Uuid::new_v4().to_string();
    let command = AuthorizeRollback {
        operation_id: Uuid::new_v4(),
        conversation_id: id,
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: note.clone(),
        },
    };
    {
        let sql = database.lock().unwrap();
        sql.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'backup note','now','now')",[&note]).unwrap();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'paired SQL receipt')",
            [id.as_uuid().to_string()],
        )
        .unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note',?2)",params![id.as_uuid().to_string(),note]).unwrap();
    }
    let receipt = database.authorize_rollback(&command).unwrap();
    assert_eq!(receipt.sql_fence.position, 0);
    assert!(!receipt.replayed);
    let expected_row = receipt_row(&database.lock().unwrap(), command.operation_id);
    assert_eq!(expected_row.0, command.operation_id.to_string());
    assert_eq!(expected_row.1, receipt.epoch.to_string());
    assert_eq!(expected_row.2, "agent.rollback_authority");
    assert_eq!(expected_row.3, 1);
    assert_eq!(expected_row.4, receipt.fingerprint);
    assert_eq!((expected_row.5, expected_row.6.as_str()), (0, "null"));
    let tail = AgentEvent::new(
        id,
        1,
        1,
        command.operation_id,
        EventPayload::ItemStarted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
        },
    )
    .unwrap();
    let append = JournalCommand::new(
        id,
        Uuid::new_v4(),
        0,
        vec![tail.clone()],
        None,
        1_790_000_000_000,
    )
    .unwrap();
    let accepted = database.write(append, WriteOptions::default()).unwrap();
    assert_eq!(accepted.position, 1);
    assert_eq!(accepted.epoch, receipt.epoch);
    // Preserve an actual accepted suffix: no materialization or shutdown here.
    drop(database);
    let source = Connection::open(&path).unwrap();
    assert_eq!(
        source
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    let source_log = Database::lock_existing_pair(&source, &log_path)
        .unwrap()
        .unwrap();
    assert_eq!(source_log.head().unwrap().accepted_position, 1);
    let future_version = MIGRATIONS.last().unwrap().0 + 1;
    // Exercise the real backup selector without registering a fictitious DDL.
    let backup = backup_before_version(
        &source,
        &root.path().join("backups"),
        Some(&source_log),
        future_version,
    )
    .unwrap()
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(backup.with_extension("pair.json")).unwrap()).unwrap();
    assert_eq!(manifest["complete"], true);
    assert_eq!(manifest["epoch"], receipt.epoch.to_string());
    assert_eq!(manifest["materializedPosition"], 0);
    assert_eq!(manifest["acceptedPosition"], 1);
    let backup_sql = Connection::open(&backup).unwrap();
    assert_eq!(receipt_row(&backup_sql, command.operation_id), expected_row);
    assert_eq!(
        backup_sql
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    let backup_log = backup.with_extension("raft");
    let locked = Database::lock_existing_pair(&backup_sql, &backup_log)
        .unwrap()
        .unwrap();
    assert_eq!(locked.head().unwrap().accepted_position, 1);
    let (sql, locked) =
        Database::prepare_schema_migration(backup_sql, &backup_log, locked).unwrap();
    let restored = Database::open_with_log(sql, &backup_log, Some(locked)).unwrap();
    let proof = match restored.rollback_authorization_status(&command).unwrap() {
        SqlImmediateStatus::Committed(proof) => proof,
        other => panic!("expected committed backup registration, got {other:?}"),
    };
    assert_eq!(proof.operation_id, receipt.operation_id);
    assert_eq!(proof.epoch, receipt.epoch);
    assert_eq!(proof.sql_fence, receipt.sql_fence);
    assert_eq!(proof.fingerprint, receipt.fingerprint);
    let retry = restored.authorize_rollback(&command).unwrap();
    assert_eq!(retry, proof);
    assert!(retry.replayed);
    {
        let mut sql = restored.lock().unwrap();
        assert_eq!(receipt_row(&sql, command.operation_id), expected_row);
        assert_eq!(
            sql.query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row(
                "select count(*) from data_sql_operation_receipts",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row("select count(*) from data_operation_receipts", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
            1
        );
        let tx = sql.transaction().unwrap();
        let page = journal_storage::read_history_in::<AgentEvent>(
            &tx,
            HistoryQuery::new(id, 0, 500).unwrap(),
        )
        .unwrap();
        assert_eq!(page.items, [tail]);
        assert_eq!(page.durable_count, 1);
        tx.commit().unwrap();
    }
    restored.shutdown().unwrap();
    drop(restored);
    let restored_log = RaftLog::open(&backup_log, Some(receipt.epoch)).unwrap();
    assert_eq!(restored_log.head().unwrap().accepted_position, 1);
    // The source pair remained exclusively owned throughout backup/recovery.
    assert!(Database::lock_existing_pair(&source, &log_path).is_err());
}
